// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build and run `tools/schema-oracle` against the pinned Tranquillity checkout.
//! Build output goes under `.scratch/`, never into the checkout.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

pub fn generate(fixture_dir: Option<PathBuf>) -> ExitCode {
    run(true, fixture_dir)
}

pub fn verify(fixture_dir: Option<PathBuf>) -> ExitCode {
    run(false, fixture_dir)
}

/// Migrate empty MySQL and PostgreSQL databases with Tranquillity's `Migration.Update`
/// and write `fixtures/db/mysql` and `fixtures/db/postgres`.
///
/// The local containers are `ferroan-q20-mysql` (port 13306) and `ferroan-q20-postgres`
/// (port 15432). `FERROAN_MYSQL_CONTAINER` and `FERROAN_POSTGRES_CONTAINER` override the names.
pub fn servers() -> ExitCode {
    let root = crate::repo_root();
    let reference = match reference_root().and_then(crate::oracle_reference::prepare) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("schema: {error}");
            return ExitCode::FAILURE;
        }
    };
    let revision = match pinned_and_checked(&root, &reference) {
        Ok(revision) => revision,
        Err(error) => {
            eprintln!("schema: {error}");
            return ExitCode::FAILURE;
        }
    };
    let artifacts = root.join(".scratch/schema-oracle/artifacts");
    if let Err(error) = fs::create_dir_all(&artifacts) {
        eprintln!("schema: could not create {}: {error}", artifacts.display());
        return ExitCode::FAILURE;
    }
    let project = root.join("tools/schema-oracle/SchemaOracle.csproj");
    if !dotnet(
        &root,
        &reference,
        &artifacts,
        &project,
        &OracleCommand {
            action: "build",
            no_build: false,
            args: &[],
            env: &[],
        },
    ) {
        return fail_if_dirty(&reference);
    }
    if !checkout_is_clean(&reference) {
        return ExitCode::FAILURE;
    }
    let mysql_container = env_or("FERROAN_MYSQL_CONTAINER", "ferroan-q20-mysql");
    let postgres_container = env_or("FERROAN_POSTGRES_CONTAINER", "ferroan-q20-postgres");
    if !reset_server_databases(&mysql_container, &postgres_container) {
        return ExitCode::FAILURE;
    }
    let fixtures = root.join("fixtures/db");
    let program = vec![
        "servers".to_string(),
        fixtures.display().to_string(),
        "--revision".to_string(),
        revision.clone(),
    ];
    let mysql_ado = ado_mysql();
    let postgres_ado = ado_postgres();
    let env = [
        ("FERROAN_SCHEMA_MYSQL", mysql_ado.as_str()),
        ("FERROAN_SCHEMA_POSTGRES", postgres_ado.as_str()),
    ];
    if !dotnet(
        &root,
        &reference,
        &artifacts,
        &project,
        &OracleCommand {
            action: "run",
            no_build: true,
            args: &program,
            env: &env,
        },
    ) {
        return fail_if_dirty(&reference);
    }
    if !checkout_is_clean(&reference) {
        return ExitCode::FAILURE;
    }
    if !dump_server_schemas(&root, &mysql_container, &postgres_container) {
        return ExitCode::FAILURE;
    }
    println!("schema: servers passed against {revision}");
    ExitCode::SUCCESS
}

fn env_or(name: &str, default: &str) -> String {
    match env::var(name) {
        Ok(value) if !value.is_empty() => value,
        _ => default.to_string(),
    }
}

fn ado_mysql() -> String {
    "Server=127.0.0.1;Port=13306;Database=ferroan;User ID=ferroan;Password=ferroan;SslMode=None;Pooling=false".to_string()
}

fn ado_postgres() -> String {
    "Server=127.0.0.1;Port=15432;Database=ferroan;User Id=ferroan;Password=ferroan;SSL Mode=Disable;Pooling=false".to_string()
}

fn reset_server_databases(mysql_container: &str, postgres_container: &str) -> bool {
    // MYSQL_PWD stays in the container environment. psql runs DROP and CREATE
    // as two commands: one `-c` string is a single transaction, and PostgreSQL
    // refuses DROP DATABASE inside a transaction block.
    let mysql = Command::new("docker")
        .args([
            "exec",
            "-e",
            "MYSQL_PWD=ferroan",
            mysql_container,
            "mysql",
            "-uroot",
            "-e",
            "DROP DATABASE IF EXISTS ferroan; CREATE DATABASE ferroan CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci; GRANT ALL ON ferroan.* TO 'ferroan'@'%'; FLUSH PRIVILEGES;",
        ])
        .status();
    if !command_ok("reset mysql", mysql) {
        return false;
    }
    let drop = Command::new("docker")
        .args([
            "exec",
            postgres_container,
            "psql",
            "-U",
            "ferroan",
            "-d",
            "postgres",
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            "DROP DATABASE IF EXISTS ferroan WITH (FORCE);",
        ])
        .status();
    if !command_ok("reset postgres drop", drop) {
        return false;
    }
    let create = Command::new("docker")
        .args([
            "exec",
            postgres_container,
            "psql",
            "-U",
            "ferroan",
            "-d",
            "postgres",
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            "CREATE DATABASE ferroan;",
        ])
        .status();
    command_ok("reset postgres create", create)
}

fn dump_server_schemas(root: &Path, mysql_container: &str, postgres_container: &str) -> bool {
    let mysql_path = root.join("fixtures/db/mysql/schema.sql");
    let postgres_path = root.join("fixtures/db/postgres/schema.sql");
    if !dump_mysql(mysql_container, &mysql_path) {
        return false;
    }
    dump_postgres(postgres_container, &postgres_path)
}

fn dump_mysql(container: &str, path: &Path) -> bool {
    let output = Command::new("docker")
        .args([
            "exec",
            "-e",
            "MYSQL_PWD=ferroan",
            container,
            "mysqldump",
            "-uferroan",
            "--no-tablespaces",
            "--skip-comments",
            "--skip-dump-date",
            "--set-gtid-purged=OFF",
            "--routines",
            "--triggers",
            "--single-transaction",
            "--skip-add-locks",
            "ferroan",
        ])
        .output();
    write_dump("mysqldump", path, output)
}

fn dump_postgres(container: &str, path: &Path) -> bool {
    let output = Command::new("docker")
        .args([
            "exec",
            "-e",
            "PGPASSWORD=ferroan",
            container,
            "pg_dump",
            "-U",
            "ferroan",
            "-d",
            "ferroan",
            "--no-owner",
            "--no-privileges",
            "--no-comments",
            "--inserts",
        ])
        .output();
    write_dump("pg_dump", path, output)
}

fn write_dump(tool: &str, path: &Path, output: std::io::Result<std::process::Output>) -> bool {
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            eprintln!("schema: could not run {tool}: {error}");
            return false;
        }
    };
    if !output.status.success() {
        eprintln!(
            "schema: {tool} exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return false;
    }
    let body = if tool == "pg_dump" {
        strip_pg_restrict(&output.stdout)
    } else {
        output.stdout.clone()
    };
    if body.len() < 64 {
        eprintln!("schema: {tool} wrote too little");
        return false;
    }
    if let Some(parent) = path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        eprintln!("schema: could not create {}: {error}", parent.display());
        return false;
    }
    if let Err(error) = fs::write(path, &body) {
        eprintln!("schema: could not write {}: {error}", path.display());
        return false;
    }
    println!("schema: wrote {} ({} bytes)", path.display(), body.len());
    true
}

/// pg_dump 16.10 writes a random `\restrict` token. It is not schema, and an
/// older client rejects the command. Drop both lines so the dump stays stable.
fn strip_pg_restrict(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    for line in input.split_inclusive(|byte| *byte == b'\n') {
        let mut bare = line;
        if let Some(stripped) = bare.strip_suffix(b"\n") {
            bare = stripped;
        }
        if let Some(stripped) = bare.strip_suffix(b"\r") {
            bare = stripped;
        }
        if bare.starts_with(b"\\restrict ") || bare.starts_with(b"\\unrestrict ") {
            continue;
        }
        out.extend_from_slice(line);
    }
    out
}

fn command_ok(action: &str, status: std::io::Result<std::process::ExitStatus>) -> bool {
    match status {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("schema: {action} exited with {status}");
            false
        }
        Err(error) => {
            eprintln!("schema: {action} failed: {error}");
            false
        }
    }
}

fn run(writing: bool, fixture_dir: Option<PathBuf>) -> ExitCode {
    let root = crate::repo_root();
    let reference = match reference_root().and_then(crate::oracle_reference::prepare) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("schema: {error}");
            return ExitCode::FAILURE;
        }
    };
    let revision = match pinned_and_checked(&root, &reference) {
        Ok(revision) => revision,
        Err(error) => {
            eprintln!("schema: {error}");
            return ExitCode::FAILURE;
        }
    };
    let artifacts = root.join(".scratch/schema-oracle/artifacts");
    if let Err(error) = fs::create_dir_all(&artifacts) {
        eprintln!("schema: could not create {}: {error}", artifacts.display());
        return ExitCode::FAILURE;
    }
    let project = root.join("tools/schema-oracle/SchemaOracle.csproj");
    let fixtures = fixture_dir.map_or_else(
        || root.join("fixtures/db/sqlite"),
        |path| {
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        },
    );
    if !dotnet(
        &root,
        &reference,
        &artifacts,
        &project,
        &OracleCommand {
            action: "build",
            no_build: false,
            args: &[],
            env: &[],
        },
    ) {
        return fail_if_dirty(&reference);
    }
    if !checkout_is_clean(&reference) {
        return ExitCode::FAILURE;
    }
    let action = if writing { "generate" } else { "verify" };
    let program = vec![
        action.to_string(),
        fixtures.display().to_string(),
        "--revision".to_string(),
        revision.clone(),
    ];
    if !dotnet(
        &root,
        &reference,
        &artifacts,
        &project,
        &OracleCommand {
            action: "run",
            no_build: true,
            args: &program,
            env: &[],
        },
    ) {
        return ExitCode::FAILURE;
    }
    if !checkout_is_clean(&reference) {
        return ExitCode::FAILURE;
    }
    println!("schema: {action} passed against {revision}");
    ExitCode::SUCCESS
}

fn reference_root() -> Result<PathBuf, String> {
    match env::var("FERROAN_REF") {
        Ok(value) if !value.is_empty() => Ok(expand_user(&value)),
        Ok(_) | Err(_) => home_dir().map(|home| home.join("Git/reference")),
    }
}

fn home_dir() -> Result<PathBuf, String> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| {
            "set FERROAN_REF, or HOME / USERPROFILE, to find ~/Git/reference".to_string()
        })
}

fn expand_user(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}

fn pinned_and_checked(root: &Path, reference: &Path) -> Result<String, String> {
    let text = fs::read_to_string(root.join("docs/REFERENCES.md"))
        .map_err(|error| format!("could not read docs/REFERENCES.md: {error}"))?;
    let pinned = pinned_revision(&text)?;
    let checkout = reference.join("OpenSim-Tranquillity");
    let project = checkout.join("Source/OpenSim.Data.SQLite/OpenSim.Data.SQLite.csproj");
    if !project.is_file() {
        return Err(format!(
            "missing {} (FERROAN_REF={})",
            project.display(),
            reference.display()
        ));
    }
    let actual = git(&checkout, &["rev-parse", "HEAD"])?;
    if actual != pinned {
        return Err(format!(
            "OpenSim-Tranquillity is at {actual}, docs/REFERENCES.md pins {pinned}"
        ));
    }
    let status = git(&checkout, &["status", "--porcelain"])?;
    if !status.is_empty() {
        return Err(format!(
            "OpenSim-Tranquillity checkout is not clean; schema generation only reads it:\n{status}"
        ));
    }
    Ok(actual)
}

fn pinned_revision(markdown: &str) -> Result<String, String> {
    for line in markdown.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("| `OpenSim-Tranquillity`") {
            continue;
        }
        let bytes = trimmed.as_bytes();
        let mut index = 0;
        while index + 42 <= bytes.len() {
            if bytes[index] == b'`' && bytes[index + 41] == b'`' {
                let hash = &trimmed[index + 1..index + 41];
                if hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Ok(hash.to_ascii_lowercase());
                }
            }
            index += 1;
        }
        return Err("OpenSim-Tranquillity row has no 40-character revision".to_string());
    }
    Err("docs/REFERENCES.md has no OpenSim-Tranquillity row".to_string())
}

fn checkout_is_clean(reference: &Path) -> bool {
    match git(
        &reference.join("OpenSim-Tranquillity"),
        &["status", "--porcelain"],
    ) {
        Ok(status) if status.is_empty() => true,
        Ok(status) => {
            eprintln!(
                "schema: the build wrote into the OpenSim-Tranquillity checkout. That checkout is read-only:\n{status}"
            );
            false
        }
        Err(error) => {
            eprintln!("schema: {error}");
            false
        }
    }
}

fn fail_if_dirty(reference: &Path) -> ExitCode {
    let _ = checkout_is_clean(reference);
    ExitCode::FAILURE
}

fn git(checkout: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(args)
        .output()
        .map_err(|error| format!("could not run git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

struct OracleCommand<'a> {
    action: &'a str,
    no_build: bool,
    args: &'a [String],
    env: &'a [(&'a str, &'a str)],
}

fn dotnet(
    root: &Path,
    reference: &Path,
    artifacts: &Path,
    project: &Path,
    run: &OracleCommand<'_>,
) -> bool {
    let mut command = crate::dotnet_ci::command("dotnet");
    command.current_dir(root).arg(run.action);
    if run.action == "run" {
        command.arg("--project").arg(project);
    } else {
        command.arg(project);
    }
    command
        .arg("-c")
        .arg("Release")
        .arg("--verbosity")
        .arg("minimal")
        .arg(format!("-p:FerroanRef={}", reference.display()))
        .arg("-p:UseArtifactsOutput=true")
        .arg(format!("-p:ArtifactsPath={}", artifacts.display()))
        .arg("-p:GeneratePackageOnBuild=false")
        .arg("-p:ContinuousIntegrationBuild=true")
        .arg(crate::dotnet_ci::CLOUD_BUILD_VERSION_VARS_OFF);
    for (key, value) in run.env {
        command.env(key, value);
    }
    if run.no_build {
        command.arg("--no-build");
    }
    if !run.args.is_empty() {
        command.arg("--");
        command.args(run.args);
    }
    let shown = project.strip_prefix(root).unwrap_or(project).display();
    if run.args.is_empty() {
        println!("$ dotnet {} -c Release {shown}", run.action);
    } else {
        println!(
            "$ dotnet {} -c Release {shown} -- {}",
            run.action,
            run.args.join(" ")
        );
    }
    match command.status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("schema: dotnet exited with {status}");
            false
        }
        Err(error) => {
            eprintln!(
                "schema: could not run dotnet (the .NET SDK is required for schema fixtures): {error}"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_tranquillity_revision() {
        let markdown = "| `OpenSim-Tranquillity` | somewhere | `713857f10b8e91a639a967d4354a4682b080793c` | note |\n";
        assert_eq!(
            pinned_revision(markdown).unwrap(),
            "713857f10b8e91a639a967d4354a4682b080793c"
        );
    }

    #[test]
    fn rejects_a_table_without_the_row() {
        assert!(
            pinned_revision("| `other` | `aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa` |\n").is_err()
        );
    }
}
