// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Throwaway MySQL and PostgreSQL servers in a non-UTC zone. Pin8 reads the
//! Presence and password-token rows Ferroan writes. The containers are removed
//! when the command returns. This does not start the live harness.

use std::io::Write;
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::Duration;

use crate::asset_oracle::{
    checkout_is_clean, dotnet, fail_if_dirty, pinned_and_checked, reference_root,
};
use crate::oracle_reference::prepare;

const MYSQL_NAME: &str = "ferroan-qr35-mysql";
const POSTGRES_NAME: &str = "ferroan-qr35-postgres";
const MYSQL_PORT: u16 = 18065;
const POSTGRES_PORT: u16 = 18064;

struct Container {
    name: &'static str,
}

impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", self.name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

pub fn run() -> ExitCode {
    let mysql = match launch_mysql() {
        Ok(container) => container,
        Err(error) => {
            eprintln!("session-zone: {error}");
            return ExitCode::FAILURE;
        }
    };
    let postgres = match launch_postgres() {
        Ok(container) => container,
        Err(error) => {
            eprintln!("session-zone: {error}");
            return ExitCode::FAILURE;
        }
    };
    let zone_dll = match build_oracle(
        "zone",
        ".scratch/zone-oracle/artifacts",
        "tools/zone-oracle/ZoneOracle.csproj",
        "bin/ZoneOracle/release/ZoneOracle.dll",
    ) {
        Ok(dll) => dll,
        Err(code) => return code,
    };
    let presence_dll = match build_oracle(
        "presence",
        ".scratch/presence-oracle/artifacts",
        "tools/presence-oracle/PresenceOracle.csproj",
        "bin/PresenceOracle/release/PresenceOracle.dll",
    ) {
        Ok(dll) => dll,
        Err(code) => return code,
    };
    if let Err(error) = wait_ready(&mysql, &postgres) {
        eprintln!("session-zone: {error}");
        return ExitCode::FAILURE;
    }
    if let Err(error) = grant_reader() {
        eprintln!("session-zone: {error}");
        return ExitCode::FAILURE;
    }
    let mysql_zone = match measured("mysql") {
        Ok(zone) => zone,
        Err(error) => {
            eprintln!("session-zone: {error}");
            return ExitCode::FAILURE;
        }
    };
    let postgres_zone = match measured("postgres") {
        Ok(zone) => zone,
        Err(error) => {
            eprintln!("session-zone: {error}");
            return ExitCode::FAILURE;
        }
    };
    let berlin = match measured("berlin") {
        Ok(zone) => zone,
        Err(error) => {
            eprintln!("session-zone: {error}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "session-zone: MySQL {} {}, PostgreSQL {} {}, Berlin override {} {}",
        mysql_zone.name,
        mysql_zone.offset,
        postgres_zone.name,
        postgres_zone.offset,
        berlin.name,
        berlin.offset
    );
    let mysql_ado = mysql_connection();
    let postgres_ado = postgres_connection(None);
    let berlin_ado = postgres_connection(Some("Europe/Berlin"));
    let unpriv_plain = unprivileged_connection(None);
    let unpriv_key = unprivileged_connection(Some("America/New_York"));
    let bad_ado = postgres_connection(Some("NotAZone"));
    if !cargo_zone(
        "mysql server zone",
        &zone_dll,
        "mysql",
        &mysql_ado,
        &mysql_zone,
        "America/New_York",
        &[],
    ) {
        return ExitCode::FAILURE;
    }
    if !cargo_zone(
        "postgres server zone",
        &zone_dll,
        "postgres",
        &postgres_ado,
        &postgres_zone,
        "America/New_York",
        &[
            ("FERROAN_ZONE_UNPRIV_ADO", unpriv_plain.as_str()),
            ("FERROAN_ZONE_UNPRIV_KEY_ADO", unpriv_key.as_str()),
            ("FERROAN_ZONE_BAD_ADO", bad_ado.as_str()),
        ],
    ) {
        return ExitCode::FAILURE;
    }
    if !cargo_zone(
        "postgres Timezone=Europe/Berlin",
        &zone_dll,
        "postgres",
        &berlin_ado,
        &berlin,
        "Europe/Berlin",
        &[],
    ) {
        return ExitCode::FAILURE;
    }
    if !cargo_presence(&presence_dll, &postgres_ado) {
        return ExitCode::FAILURE;
    }
    if let Err(error) = postgres_log_has_no_privilege_denial() {
        eprintln!("session-zone: {error}");
        return ExitCode::FAILURE;
    }
    let tokyo = match measured("tokyo") {
        Ok(zone) => zone,
        Err(error) => {
            eprintln!("session-zone: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = expect_zone(&tokyo, "Asia/Tokyo") {
        eprintln!("session-zone: {error}");
        return ExitCode::FAILURE;
    }
    if let Err(error) = psql("ALTER DATABASE ferroan SET timezone TO 'Europe/Berlin'") {
        eprintln!("session-zone: {error}");
        return ExitCode::FAILURE;
    }
    let database_zone = match measured("postgres") {
        Ok(zone) => zone,
        Err(error) => {
            eprintln!("session-zone: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = expect_zone(&database_zone, "Europe/Berlin") {
        eprintln!("session-zone: {error}");
        return ExitCode::FAILURE;
    }
    if !cargo_zone(
        "postgres ALTER DATABASE timezone=Europe/Berlin",
        &zone_dll,
        "postgres",
        &postgres_ado,
        &database_zone,
        "Europe/Berlin",
        &[],
    ) {
        return ExitCode::FAILURE;
    }
    if !cargo_zone(
        "postgres PGTZ=Asia/Tokyo",
        &zone_dll,
        "postgres",
        &postgres_ado,
        &tokyo,
        "Asia/Tokyo",
        &[("PGTZ", "Asia/Tokyo")],
    ) {
        return ExitCode::FAILURE;
    }
    if let Err(error) = psql("ALTER ROLE ferroan SET timezone TO 'Asia/Tokyo'") {
        eprintln!("session-zone: {error}");
        return ExitCode::FAILURE;
    }
    let role_zone = match measured("postgres") {
        Ok(zone) => zone,
        Err(error) => {
            eprintln!("session-zone: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = expect_zone(&role_zone, "Asia/Tokyo") {
        eprintln!("session-zone: {error}");
        return ExitCode::FAILURE;
    }
    if !cargo_zone(
        "postgres ALTER ROLE timezone=Asia/Tokyo",
        &zone_dll,
        "postgres",
        &postgres_ado,
        &role_zone,
        "Asia/Tokyo",
        &[],
    ) {
        return ExitCode::FAILURE;
    }
    if let Err(error) =
        psql("ALTER ROLE ferroan IN DATABASE ferroan SET timezone TO 'America/New_York'")
    {
        eprintln!("session-zone: {error}");
        return ExitCode::FAILURE;
    }
    let both_zone = match measured("postgres") {
        Ok(zone) => zone,
        Err(error) => {
            eprintln!("session-zone: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = expect_zone(&both_zone, "America/New_York") {
        eprintln!("session-zone: {error}");
        return ExitCode::FAILURE;
    }
    if !cargo_zone(
        "postgres ALTER ROLE IN DATABASE timezone=America/New_York",
        &zone_dll,
        "postgres",
        &postgres_ado,
        &both_zone,
        "America/New_York",
        &[],
    ) {
        return ExitCode::FAILURE;
    }
    if let Err(error) = postgres_log_has_no_privilege_denial() {
        eprintln!("session-zone: {error}");
        return ExitCode::FAILURE;
    }
    println!("session-zone: Pin8 read Ferroan's rows on both servers");
    ExitCode::SUCCESS
}

struct Measured {
    name: String,
    offset: String,
}

fn measured(kind: &str) -> Result<Measured, String> {
    let output = match kind {
        "mysql" => Command::new("docker")
            .args([
                "exec",
                MYSQL_NAME,
                "mysql",
                "-N",
                "-B",
                "-uroot",
                "-pferroan",
                "-e",
                "SELECT CONCAT(@@global.time_zone, ' ', TIMESTAMPDIFF(MINUTE, UTC_TIMESTAMP(), NOW()))",
            ])
            .output(),
        "postgres" => Command::new("docker")
            .args([
                "exec",
                POSTGRES_NAME,
                "psql",
                "-U",
                "ferroan",
                "-d",
                "ferroan",
                "-tA",
                "-c",
                "SELECT current_setting('TimeZone') || ' ' || ((EXTRACT(TIMEZONE FROM now()) / 60)::integer)::text",
            ])
            .output(),
        "berlin" => Command::new("docker")
            .args([
                "exec",
                POSTGRES_NAME,
                "psql",
                "-U",
                "ferroan",
                "-d",
                "ferroan",
                "-tA",
                "-c",
                "BEGIN; SET TIME ZONE 'Europe/Berlin'; SELECT current_setting('TimeZone') || ' ' || ((EXTRACT(TIMEZONE FROM now()) / 60)::integer)::text; ROLLBACK;",
            ])
            .output(),
        "tokyo" => Command::new("docker")
            .args([
                "exec",
                POSTGRES_NAME,
                "psql",
                "-U",
                "ferroan",
                "-d",
                "ferroan",
                "-tA",
                "-c",
                "BEGIN; SET TIME ZONE 'Asia/Tokyo'; SELECT current_setting('TimeZone') || ' ' || ((EXTRACT(TIMEZONE FROM now()) / 60)::integer)::text; ROLLBACK;",
            ])
            .output(),
        _ => return Err(format!("unknown zone probe {kind}")),
    }
    .map_err(|error| format!("could not read the {kind} zone: {error}"))?;
    if !output.status.success() {
        return Err(format!("could not read the {kind} zone"));
    }
    parse_measured(kind, &String::from_utf8_lossy(&output.stdout))
}

fn parse_measured(kind: &str, text: &str) -> Result<Measured, String> {
    for line in text.lines().rev() {
        let line = line.trim();
        let Some((name, offset)) = line.rsplit_once(' ') else {
            continue;
        };
        if name.is_empty() || offset.parse::<i64>().is_err() {
            continue;
        }
        return Ok(Measured {
            name: name.to_string(),
            offset: offset.to_string(),
        });
    }
    Err(format!("{kind} zone probe returned nothing usable"))
}

fn cargo_zone(
    label: &str,
    dll: &std::path::Path,
    engine: &str,
    ado: &str,
    zone: &Measured,
    process_tz: &str,
    extra: &[(&str, &str)],
) -> bool {
    let mut command = Command::new("cargo");
    command
        .current_dir(crate::repo_root())
        .env_remove("FERROAN_MYSQL_URL")
        .env_remove("FERROAN_POSTGRES_URL")
        .env_remove("DATABASE_URL")
        .env("TZ", process_tz)
        .env("FERROAN_ZONE_ORACLE", dll)
        .env("FERROAN_ZONE_ENGINE", engine)
        .env("FERROAN_ZONE_ADO", ado)
        .env("FERROAN_ZONE_NAME", &zone.name)
        .env("FERROAN_ZONE_OFFSET", &zone.offset)
        .env_remove("PGTZ");
    for (key, value) in extra {
        command.env(key, value);
    }
    command.args([
        "test",
        "-p",
        "ferroan-services",
        "--lib",
        "session_zone_csharp_reads_ferroan_rows",
        "--",
        "--nocapture",
    ]);
    println!(
        "$ cargo test -p ferroan-services --lib session_zone_csharp_reads_ferroan_rows ({label})"
    );
    match command.status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("session-zone: {label} exited with {status}");
            false
        }
        Err(error) => {
            eprintln!("session-zone: could not run cargo: {error}");
            false
        }
    }
}

fn cargo_presence(dll: &std::path::Path, ado: &str) -> bool {
    let mut command = Command::new("cargo");
    command
        .current_dir(crate::repo_root())
        .env_remove("FERROAN_MYSQL_URL")
        .env_remove("FERROAN_POSTGRES_URL")
        .env_remove("DATABASE_URL")
        .env("TZ", "America/New_York")
        .env("FERROAN_PRESENCE_PG_CROSS", "1")
        .env("FERROAN_PRESENCE_ORACLE", dll)
        .env("FERROAN_PRESENCE_ADO_POSTGRES", ado)
        .env_remove("PGTZ")
        .args([
            "test",
            "-p",
            "ferroan-services",
            "--lib",
            "postgres_presence_plugins_cross_read",
            "--",
            "--nocapture",
        ]);
    println!(
        "$ cargo test -p ferroan-services --lib postgres_presence_plugins_cross_read (non-UTC server)"
    );
    match command.status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("session-zone: presence cross exited with {status}");
            false
        }
        Err(error) => {
            eprintln!("session-zone: could not run cargo: {error}");
            false
        }
    }
}

fn build_oracle(
    label: &str,
    artifacts: &str,
    project: &str,
    dll: &str,
) -> Result<std::path::PathBuf, ExitCode> {
    let root = crate::repo_root();
    let reference = match reference_root().and_then(prepare) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("session-zone: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    if let Err(error) = pinned_and_checked(&root, &reference) {
        eprintln!("session-zone: {error}");
        return Err(ExitCode::FAILURE);
    }
    let artifacts = root.join(artifacts);
    if let Err(error) = std::fs::create_dir_all(&artifacts) {
        eprintln!(
            "session-zone: could not create {}: {error}",
            artifacts.display()
        );
        return Err(ExitCode::FAILURE);
    }
    let project = root.join(project);
    if !dotnet(
        label,
        &root,
        &reference,
        &artifacts,
        &project,
        "build",
        false,
        &[],
    ) {
        return Err(fail_if_dirty(label, &reference));
    }
    if !checkout_is_clean(label, &reference) {
        return Err(ExitCode::FAILURE);
    }
    let dll = artifacts.join(dll);
    if !dll.is_file() {
        eprintln!("session-zone: missing {}", dll.display());
        return Err(ExitCode::FAILURE);
    }
    Ok(dll)
}

fn launch_mysql() -> Result<Container, String> {
    launch(
        MYSQL_NAME,
        &[
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
            "-e",
            "TZ=America/New_York",
            "-p",
            &format!("127.0.0.1:{MYSQL_PORT}:3306"),
            "mysql:8.4",
            "--character-set-server=utf8mb3",
            "--collation-server=utf8mb3_general_ci",
            "--default-time-zone=-05:00",
        ],
        &format!(
            "$ docker run --name {MYSQL_NAME} -e TZ=America/New_York -p 127.0.0.1:{MYSQL_PORT}:3306 mysql:8.4 --default-time-zone=-05:00"
        ),
    )
}

fn launch_postgres() -> Result<Container, String> {
    launch(
        POSTGRES_NAME,
        &[
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
            "-e",
            "TZ=America/New_York",
            "-p",
            &format!("127.0.0.1:{POSTGRES_PORT}:5432"),
            "postgres:16",
            "-c",
            "timezone=America/New_York",
        ],
        &format!(
            "$ docker run --name {POSTGRES_NAME} -e TZ=America/New_York -p 127.0.0.1:{POSTGRES_PORT}:5432 postgres:16 -c timezone=America/New_York"
        ),
    )
}

fn launch(name: &'static str, args: &[&str], shown: &str) -> Result<Container, String> {
    let _ = Command::new("docker")
        .args(["rm", "-f", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    println!("{shown}");
    let status = Command::new("docker")
        .args(args)
        .stdout(Stdio::null())
        .status()
        .map_err(|error| format!("could not run docker: {error}"))?;
    if !status.success() {
        let _ = Command::new("docker")
            .args(["rm", "-f", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        return Err(format!("docker run exited with {status}"));
    }
    Ok(Container { name })
}

fn wait_ready(mysql: &Container, postgres: &Container) -> Result<(), String> {
    for _ in 0..90 {
        let mysql_ready = Command::new("docker")
            .args([
                "exec",
                mysql.name,
                "mysqladmin",
                "ping",
                "--protocol=TCP",
                "-h127.0.0.1",
                "-uroot",
                "-pferroan",
                "--silent",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let postgres_ready = Command::new("docker")
            .args([
                "exec",
                postgres.name,
                "pg_isready",
                "-U",
                "ferroan",
                "-d",
                "ferroan",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if matches!(mysql_ready, Ok(status) if status.success())
            && matches!(postgres_ready, Ok(status) if status.success())
        {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }
    Err("a database container did not become ready".to_string())
}

fn grant_reader() -> Result<(), String> {
    psql(
        "\
DO $$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'zone_reader') THEN
    CREATE ROLE zone_reader LOGIN PASSWORD 'zone_reader';
  END IF;
END $$;
GRANT CONNECT ON DATABASE ferroan TO zone_reader;
",
    )
}

fn psql(sql: &str) -> Result<(), String> {
    let mut child = Command::new("docker")
        .args([
            "exec",
            "-i",
            POSTGRES_NAME,
            "psql",
            "-U",
            "ferroan",
            "-d",
            "ferroan",
            "-v",
            "ON_ERROR_STOP=1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not run psql: {error}"))?;
    let Some(mut stdin) = child.stdin.take() else {
        return Err("could not write the SQL statement".to_string());
    };
    stdin
        .write_all(sql.as_bytes())
        .map_err(|error| format!("could not write the SQL statement: {error}"))?;
    drop(stdin);
    let output = child
        .wait_with_output()
        .map_err(|error| format!("could not run psql: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.to_ascii_lowercase().contains("password") {
            Err(format!("psql failed ({})", output.status))
        } else {
            Err(format!("psql failed: {}", stderr.trim()))
        }
    }
}

fn expect_zone(zone: &Measured, name: &str) -> Result<(), String> {
    if zone.name == name {
        Ok(())
    } else {
        Err(format!(
            "psql reported {}, not the expected {name}",
            zone.name
        ))
    }
}

fn postgres_log_has_no_privilege_denial() -> Result<(), String> {
    let output = Command::new("docker")
        .args(["logs", POSTGRES_NAME])
        .output()
        .map_err(|error| format!("could not read the PostgreSQL log: {error}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if let Some(line) = text
        .lines()
        .find(|line| line.to_ascii_lowercase().contains("permission denied"))
    {
        let line: String = line.chars().take(200).collect();
        Err(format!(
            "the PostgreSQL log recorded a permission denial from the zone probe: {line}"
        ))
    } else {
        Ok(())
    }
}

fn mysql_connection() -> String {
    format!(
        "Server=127.0.0.1;Port={MYSQL_PORT};Database=ferroan;User ID=ferroan;Password=ferroan;SslMode=None"
    )
}

fn postgres_connection(timezone: Option<&str>) -> String {
    let mut text = format!(
        "Host=127.0.0.1;Port={POSTGRES_PORT};Database=ferroan;Username=ferroan;Password=ferroan;SSL Mode=Disable"
    );
    if let Some(timezone) = timezone {
        text.push_str(";Timezone=");
        text.push_str(timezone);
    }
    text
}

fn unprivileged_connection(timezone: Option<&str>) -> String {
    let mut text = format!(
        "Host=127.0.0.1;Port={POSTGRES_PORT};Database=ferroan;Username=zone_reader;Password=zone_reader;SSL Mode=Disable"
    );
    if let Some(timezone) = timezone {
        text.push_str(";Timezone=");
        text.push_str(timezone);
    }
    text
}
