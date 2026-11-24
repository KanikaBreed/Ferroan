// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build `tools/regionstore-oracle` against a task-owned clone of the pin and
//! compare its object dump with the committed fixture. SQLite is a file.
//! MySQL and PostgreSQL are throwaway containers this command removes. The
//! shared reference checkout is only read.

use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::Duration;

use crate::asset_oracle::{
    checkout_is_clean, dotnet, fail_if_dirty, git, pinned_and_checked, pinned_revision,
    reference_root,
};
use crate::oracle_reference::prepare;

const LABEL: &str = "regionstore";
const ARTIFACTS: &str = ".scratch/regionstore-oracle/artifacts";
const PROJECT: &str = "tools/regionstore-oracle/RegionStoreOracle.csproj";
const DLL: &str = "bin/RegionStoreOracle/release/RegionStoreOracle.dll";
const FIXTURE: &str = "fixtures/regionstore/objects.db";
const DUMP: &str = "fixtures/regionstore/objects.json";
const SETTINGS_FIXTURE: &str = "fixtures/regionstore/settings.db";
const SETTINGS_DUMP: &str = "fixtures/regionstore/settings.json";
const XML_DUMP: &str = "fixtures/regionstore/objects-xml.json";
const XML_READ_DUMP: &str = "fixtures/regionstore/objects-xml-read.json";
const MYSQL_DUMP: &str = "fixtures/regionstore/objects-mysql.json";
const POSTGRES_DUMP: &str = "fixtures/regionstore/objects-postgres.json";
const REGION: &str = "11111111-1111-1111-1111-111111111111";
const MYSQL_NAME: &str = "ferroan-regionstore-mysql";
const MYSQL_PORT: &str = "13317";
const POSTGRES_NAME: &str = "ferroan-regionstore-postgres";
const POSTGRES_PORT: &str = "15433";

pub fn generate() -> ExitCode {
    dispatch(true)
}

pub fn verify() -> ExitCode {
    dispatch(false)
}

/// Seed a SQLite region store and write the settings dump for one region.
pub fn settings() -> ExitCode {
    let root = crate::repo_root();
    let dll = match build_oracle() {
        Ok(path) => path,
        Err(code) => return code,
    };
    let pinned = match fs::read_to_string(root.join("docs/REFERENCES.md"))
        .map_err(|error| error.to_string())
        .and_then(|text| pinned_revision(&text))
    {
        Ok(pin) => pin,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let db = root.join(SETTINGS_FIXTURE);
    let dump = root.join(SETTINGS_DUMP);
    if let Some(parent) = db.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        eprintln!("{LABEL}: could not create {}: {error}", parent.display());
        return ExitCode::FAILURE;
    }
    if db.exists()
        && let Err(error) = fs::remove_file(&db)
    {
        eprintln!("{LABEL}: could not replace {}: {error}", db.display());
        return ExitCode::FAILURE;
    }
    let db_arg = db.display().to_string();
    if !oracle(&dll, &["settings-seed", &db_arg]) {
        return ExitCode::FAILURE;
    }
    let json = match oracle_stdout(&dll, &["settings-load", &db_arg, REGION, &pinned]) {
        Some(text) => text,
        None => return ExitCode::FAILURE,
    };
    if let Err(error) = fs::write(&dump, json) {
        eprintln!("{LABEL}: could not write {}: {error}", dump.display());
        return ExitCode::FAILURE;
    }
    println!("{LABEL}: wrote {SETTINGS_FIXTURE} and {SETTINGS_DUMP}");
    ExitCode::SUCCESS
}

/// Load the committed SQLite object fixture and write each group's XML2 and
/// original XML from the pin's serializer.
pub fn serialize() -> ExitCode {
    let root = crate::repo_root();
    let dll = match build_oracle() {
        Ok(path) => path,
        Err(code) => return code,
    };
    let pinned = match fs::read_to_string(root.join("docs/REFERENCES.md"))
        .map_err(|error| error.to_string())
        .and_then(|text| pinned_revision(&text))
    {
        Ok(pin) => pin,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let db = root.join(FIXTURE);
    if !db.is_file() {
        eprintln!("{LABEL}: missing {FIXTURE}");
        return ExitCode::FAILURE;
    }
    let db_arg = db.display().to_string();
    let json = match oracle_stdout(&dll, &["serialize", &db_arg, REGION, &pinned]) {
        Some(text) => text,
        None => return ExitCode::FAILURE,
    };
    let dump = root.join(XML_DUMP);
    if let Err(error) = fs::write(&dump, json) {
        eprintln!("{LABEL}: could not write {}: {error}", dump.display());
        return ExitCode::FAILURE;
    }
    println!("{LABEL}: wrote {XML_DUMP}");
    ExitCode::SUCCESS
}

/// Read each group in the serializer fixture and write the XML C# produces
/// from the objects it read. One group also carries an unknown part element
/// and an unknown shape element; both are skipped.
pub fn read() -> ExitCode {
    let root = crate::repo_root();
    let dll = match build_oracle() {
        Ok(path) => path,
        Err(code) => return code,
    };
    let pinned = match fs::read_to_string(root.join("docs/REFERENCES.md"))
        .map_err(|error| error.to_string())
        .and_then(|text| pinned_revision(&text))
    {
        Ok(pin) => pin,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let source = root.join(XML_DUMP);
    if !source.is_file() {
        eprintln!("{LABEL}: missing {XML_DUMP}");
        return ExitCode::FAILURE;
    }
    let source_arg = source.display().to_string();
    let json = match oracle_stdout(&dll, &["read", &source_arg, &pinned]) {
        Some(text) => text,
        None => return ExitCode::FAILURE,
    };
    let dump = root.join(XML_READ_DUMP);
    if let Err(error) = fs::write(&dump, json) {
        eprintln!("{LABEL}: could not write {}: {error}", dump.display());
        return ExitCode::FAILURE;
    }
    println!("{LABEL}: wrote {XML_READ_DUMP}");
    ExitCode::SUCCESS
}

pub fn bulk(db: &str, region: &str, count: &str) -> ExitCode {
    let dll = match build_oracle() {
        Ok(path) => path,
        Err(code) => return code,
    };
    if oracle(&dll, &["bulk", db, region, count]) {
        println!("{LABEL}: wrote {count} prims for {region} to {db}");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// SQLite fixture compare, then the same groups from throwaway MySQL and
/// PostgreSQL containers. Both containers are removed when this returns.
pub fn cross() -> ExitCode {
    let root = crate::repo_root();
    let dll = match build_oracle() {
        Ok(path) => path,
        Err(code) => return code,
    };
    if verify_fixture(&root, &dll) != ExitCode::SUCCESS {
        return ExitCode::FAILURE;
    }
    let pinned = match fs::read_to_string(root.join("docs/REFERENCES.md"))
        .map_err(|error| error.to_string())
        .and_then(|text| pinned_revision(&text))
    {
        Ok(pin) => pin,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mysql = match start_mysql() {
        Ok(container) => container,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mysql_code = mysql_cross(&root, &dll, &pinned);
    drop(mysql);
    if mysql_code != ExitCode::SUCCESS {
        return mysql_code;
    }
    let postgres = match start_postgres() {
        Ok(container) => container,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let code = postgres_cross(&root, &dll, &pinned);
    drop(postgres);
    code
}

struct MysqlContainer;

impl Drop for MysqlContainer {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", MYSQL_NAME])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn start_mysql() -> Result<MysqlContainer, String> {
    let _ = Command::new("docker")
        .args(["rm", "-f", MYSQL_NAME])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    println!("$ docker run --name {MYSQL_NAME} -p 127.0.0.1:{MYSQL_PORT}:3306 mysql:8.4");
    let status = Command::new("docker")
        .args([
            "run",
            "-d",
            "--name",
            MYSQL_NAME,
            "-e",
            "MYSQL_ROOT_PASSWORD=ferroan",
            "-e",
            "MYSQL_DATABASE=ferroan",
            "-e",
            "MYSQL_USER=ferroan",
            "-e",
            "MYSQL_PASSWORD=ferroan",
            "-p",
            &format!("127.0.0.1:{MYSQL_PORT}:3306"),
            "mysql:8.4",
            "--character-set-server=utf8mb3",
            "--collation-server=utf8mb3_general_ci",
        ])
        .stdout(Stdio::null())
        .status()
        .map_err(|error| format!("could not run docker: {error}"))?;
    if !status.success() {
        let _ = Command::new("docker")
            .args(["rm", "-f", MYSQL_NAME])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        return Err(format!("docker run exited with {status}"));
    }
    let container = MysqlContainer;
    for _ in 0..60 {
        let ready = Command::new("docker")
            .args([
                "exec",
                MYSQL_NAME,
                "mysqladmin",
                "ping",
                "-uroot",
                "-pferroan",
                "--silent",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if matches!(ready, Ok(status) if status.success()) {
            return Ok(container);
        }
        thread::sleep(Duration::from_secs(1));
    }
    Err("MySQL container did not become ready".to_owned())
}

struct PostgresContainer;

impl Drop for PostgresContainer {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", POSTGRES_NAME])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn start_postgres() -> Result<PostgresContainer, String> {
    let _ = Command::new("docker")
        .args(["rm", "-f", POSTGRES_NAME])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    println!("$ docker run --name {POSTGRES_NAME} -p 127.0.0.1:{POSTGRES_PORT}:5432 postgres:16");
    let status = Command::new("docker")
        .args([
            "run",
            "-d",
            "--name",
            POSTGRES_NAME,
            "-e",
            "POSTGRES_USER=ferroan",
            "-e",
            "POSTGRES_PASSWORD=ferroan",
            "-e",
            "POSTGRES_DB=ferroan",
            "-p",
            &format!("127.0.0.1:{POSTGRES_PORT}:5432"),
            "postgres:16",
        ])
        .stdout(Stdio::null())
        .status()
        .map_err(|error| format!("could not run docker: {error}"))?;
    if !status.success() {
        let _ = Command::new("docker")
            .args(["rm", "-f", POSTGRES_NAME])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        return Err(format!("docker run exited with {status}"));
    }
    let container = PostgresContainer;
    for _ in 0..60 {
        let ready = Command::new("docker")
            .args([
                "exec",
                POSTGRES_NAME,
                "pg_isready",
                "-U",
                "ferroan",
                "-d",
                "ferroan",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if matches!(ready, Ok(status) if status.success()) {
            return Ok(container);
        }
        thread::sleep(Duration::from_secs(1));
    }
    Err("PostgreSQL container did not become ready".to_owned())
}

fn postgres_urls() -> (String, String) {
    let csharp = format!(
        "Host=127.0.0.1;Port={POSTGRES_PORT};Database=ferroan;Username=ferroan;Password=ferroan;SSL Mode=Disable;Pooling=false"
    );
    let rust = format!(
        "Host=127.0.0.1;Port={POSTGRES_PORT};Database=ferroan;Username=ferroan;Password=ferroan;SSL Mode=Disable"
    );
    (csharp, rust)
}

fn postgres_cross(root: &Path, dll: &Path, pin: &str) -> ExitCode {
    let (csharp, rust_url) = postgres_urls();
    if run_oracle(dll, "postgres-seed", &["postgres-seed", &csharp]).is_none() {
        return ExitCode::FAILURE;
    }
    let fresh = match run_oracle(
        dll,
        "postgres-load",
        &["postgres-load", &csharp, REGION, pin],
    ) {
        Some(text) => text,
        None => return ExitCode::FAILURE,
    };
    let dump = root.join(POSTGRES_DUMP);
    let committed = match fs::read_to_string(&dump) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("{LABEL}: could not read {}: {error}", dump.display());
            return ExitCode::FAILURE;
        }
    };
    if fresh != committed {
        eprintln!(
            "{LABEL}: PostgreSQL oracle dump differs from {}",
            POSTGRES_DUMP
        );
        return ExitCode::FAILURE;
    }
    println!("{LABEL}: PostgreSQL oracle dump matches {POSTGRES_DUMP}");
    if postgres_tests(root, &rust_url) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn postgres_tests(root: &Path, connection: &str) -> bool {
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .env("FERROAN_REGIONSTORE", "1")
        .env("FERROAN_REGIONSTORE_CROSS", "1")
        .env("FERROAN_REGIONSTORE_POSTGRES", connection)
        .args([
            "test",
            "-p",
            "ferroan-scene",
            "--lib",
            "regionstore_postgres",
            "--",
            "--nocapture",
        ]);
    println!("$ cargo test -p ferroan-scene --lib regionstore_postgres");
    match command.status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("{LABEL}: PostgreSQL fixture tests exited with {status}");
            false
        }
        Err(error) => {
            eprintln!("{LABEL}: could not run cargo: {error}");
            false
        }
    }
}

fn mysql_urls() -> (String, String) {
    let csharp = format!(
        "Server=127.0.0.1;Port={MYSQL_PORT};Database=ferroan;User ID=ferroan;Password=ferroan;SslMode=None;Pooling=false"
    );
    let rust = format!(
        "Server=127.0.0.1;Port={MYSQL_PORT};Database=ferroan;User ID=ferroan;Password=ferroan;SslMode=None"
    );
    (csharp, rust)
}

fn mysql_cross(root: &Path, dll: &Path, pin: &str) -> ExitCode {
    let (csharp, rust_url) = mysql_urls();
    if run_oracle(dll, "mysql-seed", &["mysql-seed", &csharp]).is_none() {
        return ExitCode::FAILURE;
    }
    let fresh = match run_oracle(dll, "mysql-load", &["mysql-load", &csharp, REGION, pin]) {
        Some(text) => text,
        None => return ExitCode::FAILURE,
    };
    let dump = root.join(MYSQL_DUMP);
    let committed = match fs::read_to_string(&dump) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("{LABEL}: could not read {}: {error}", dump.display());
            return ExitCode::FAILURE;
        }
    };
    if fresh != committed {
        eprintln!("{LABEL}: MySQL oracle dump differs from {}", MYSQL_DUMP);
        return ExitCode::FAILURE;
    }
    println!("{LABEL}: MySQL oracle dump matches {MYSQL_DUMP}");
    if mysql_tests(root, dll, &rust_url) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn mysql_tests(root: &Path, dll: &Path, connection: &str) -> bool {
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .env("FERROAN_REGIONSTORE", "1")
        .env("FERROAN_REGIONSTORE_ORACLE", dll)
        .env("FERROAN_REGIONSTORE_CROSS", "1")
        .env("FERROAN_REGIONSTORE_MYSQL", connection)
        .args([
            "test",
            "-p",
            "ferroan-scene",
            "--lib",
            "regionstore_",
            "--",
            "--nocapture",
        ]);
    println!("$ cargo test -p ferroan-scene --lib regionstore_");
    match command.status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("{LABEL}: MySQL fixture tests exited with {status}");
            false
        }
        Err(error) => {
            eprintln!("{LABEL}: could not run cargo: {error}");
            false
        }
    }
}

fn run_oracle(dll: &Path, label: &str, args: &[&str]) -> Option<String> {
    println!("$ dotnet {} {label}", dll.display());
    let mut command = Command::new("dotnet");
    command
        .arg(dll)
        .args(args)
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("MSBUILDDISABLENODEREUSE", "1");
    match command.output() {
        Ok(output) if output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.trim().is_empty() {
                eprint!("{stderr}");
            }
            Some(String::from_utf8_lossy(&output.stdout).into_owned())
        }
        Ok(output) => {
            eprintln!(
                "{LABEL}: {label} exited with {}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
            None
        }
        Err(error) => {
            eprintln!("{LABEL}: could not run dotnet: {error}");
            None
        }
    }
}

fn dispatch(writing: bool) -> ExitCode {
    let root = crate::repo_root();
    let dll = match build_oracle() {
        Ok(path) => path,
        Err(code) => return code,
    };
    let pinned = match fs::read_to_string(root.join("docs/REFERENCES.md"))
        .map_err(|error| error.to_string())
        .and_then(|text| pinned_revision(&text))
    {
        Ok(pin) => pin,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return ExitCode::FAILURE;
        }
    };
    if writing {
        return write_fixture(&root, &dll, &pinned);
    }
    verify_fixture(&root, &dll)
}

fn build_oracle() -> Result<std::path::PathBuf, ExitCode> {
    let root = crate::repo_root();
    let source = match reference_root() {
        Ok(path) => path.join("OpenSim-Tranquillity"),
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    let source_head = match git(&source, &["rev-parse", "HEAD"]) {
        Ok(head) => head,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    let reference = match prepare(source.parent().unwrap_or(&source).to_path_buf()) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    let pinned = match fs::read_to_string(root.join("docs/REFERENCES.md"))
        .map_err(|error| error.to_string())
        .and_then(|text| pinned_revision(&text))
    {
        Ok(pin) => pin,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    let clone = reference.join("OpenSim-Tranquillity");
    if let Err(error) = detach_at_pin(&clone, &source, &pinned) {
        eprintln!("{LABEL}: {error}");
        return Err(ExitCode::FAILURE);
    }
    if let Err(error) = pinned_and_checked(&root, &reference) {
        eprintln!("{LABEL}: {error}");
        return Err(ExitCode::FAILURE);
    }
    let artifacts = root.join(ARTIFACTS);
    if let Err(error) = fs::create_dir_all(&artifacts) {
        eprintln!("{LABEL}: could not create {}: {error}", artifacts.display());
        return Err(ExitCode::FAILURE);
    }
    let project = root.join(PROJECT);
    if !dotnet(
        LABEL,
        &root,
        &reference,
        &artifacts,
        &project,
        "build",
        false,
        &[],
    ) {
        return Err(fail_if_dirty(LABEL, &reference));
    }
    if !checkout_is_clean(LABEL, &reference) {
        return Err(ExitCode::FAILURE);
    }
    if !source_unchanged(&source, &source_head) {
        return Err(ExitCode::FAILURE);
    }
    let dll = artifacts.join(DLL);
    if !dll.is_file() {
        eprintln!("{LABEL}: missing {}", dll.display());
        return Err(ExitCode::FAILURE);
    }
    Ok(dll)
}

fn detach_at_pin(clone: &Path, source: &Path, pin: &str) -> Result<(), String> {
    if git(clone, &["cat-file", "-e", &format!("{pin}^{{commit}}")]).is_err() {
        let remote = source.to_str().ok_or("reference path is not UTF-8")?;
        git(clone, &["fetch", remote, pin])?;
    }
    git(clone, &["checkout", "--detach", pin])?;
    Ok(())
}

fn source_unchanged(source: &Path, head: &str) -> bool {
    match git(source, &["rev-parse", "HEAD"]) {
        Ok(now) if now == head => {}
        Ok(now) => {
            eprintln!("{LABEL}: the reference checkout moved from {head} to {now}");
            return false;
        }
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return false;
        }
    }
    match git(source, &["status", "--porcelain"]) {
        Ok(status) if status.is_empty() => true,
        Ok(status) => {
            eprintln!("{LABEL}: the reference checkout is no longer clean:\n{status}");
            false
        }
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            false
        }
    }
}

fn write_fixture(root: &Path, dll: &Path, pin: &str) -> ExitCode {
    let db = root.join(FIXTURE);
    let dump = root.join(DUMP);
    if let Some(parent) = db.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        eprintln!("{LABEL}: could not create {}: {error}", parent.display());
        return ExitCode::FAILURE;
    }
    if db.exists()
        && let Err(error) = fs::remove_file(&db)
    {
        eprintln!("{LABEL}: could not replace {}: {error}", db.display());
        return ExitCode::FAILURE;
    }
    let db_arg = db.display().to_string();
    if !oracle(dll, &["seed", &db_arg]) {
        return ExitCode::FAILURE;
    }
    let json = match oracle_stdout(dll, &["load", &db_arg, REGION, pin]) {
        Some(text) => text,
        None => return ExitCode::FAILURE,
    };
    if let Err(error) = fs::write(&dump, json) {
        eprintln!("{LABEL}: could not write {}: {error}", dump.display());
        return ExitCode::FAILURE;
    }
    println!("{LABEL}: wrote {} and {}", FIXTURE, DUMP);
    ExitCode::SUCCESS
}

fn verify_fixture(root: &Path, dll: &Path) -> ExitCode {
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .env("FERROAN_REGIONSTORE", "1")
        .env("FERROAN_REGIONSTORE_ORACLE", dll)
        .args([
            "test",
            "-p",
            "ferroan-scene",
            "--lib",
            "regionstore_",
            "--",
            "--nocapture",
        ]);
    println!("$ cargo test -p ferroan-scene --lib regionstore_");
    match command.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("{LABEL}: fixture tests exited with {status}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("{LABEL}: could not run cargo: {error}");
            ExitCode::FAILURE
        }
    }
}

fn oracle(dll: &Path, args: &[&str]) -> bool {
    oracle_stdout(dll, args).is_some()
}

fn oracle_stdout(dll: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new("dotnet");
    command
        .arg(dll)
        .args(args)
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("MSBUILDDISABLENODEREUSE", "1");
    println!("$ dotnet {} {}", dll.display(), args.join(" "));
    match command.output() {
        Ok(output) if output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.trim().is_empty() {
                eprint!("{stderr}");
            }
            Some(String::from_utf8_lossy(&output.stdout).into_owned())
        }
        Ok(output) => {
            eprintln!(
                "{LABEL}: oracle exited with {}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
            None
        }
        Err(error) => {
            eprintln!("{LABEL}: could not run dotnet: {error}");
            None
        }
    }
}
