// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Arguments for `cargo xtask live`.
//!
//! Grid mode takes `--rust <service>,…` and `--db sqlite|mysql|pgsql`.
//! P1.0 serves `assets` on SQLite. MySQL and PostgreSQL stay refused until
//! P1.A1 (#59) provides those stores.

use std::net::Ipv4Addr;
use std::process::ExitCode;

use crate::grid;
use crate::live;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Database {
    Sqlite,
    Mysql,
    Pgsql,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AssetBackend {
    Stock,
    Xasset,
    Fsassets,
}

impl Database {
    pub(crate) fn require_ready(self) -> Result<(), String> {
        match self {
            Database::Sqlite => Ok(()),
            Database::Mysql => {
                Err("mysql needs P1.A1 (#59) before the harness can use it".to_string())
            }
            Database::Pgsql => {
                Err("pgsql needs P1.A1 (#59) before the harness can use it".to_string())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunSpec {
    pub scenario: String,
    pub grid: bool,
    pub ferroan_region: bool,
    pub rust_services: Vec<String>,
    pub db: Database,
    /// When set, each C# region ini selects BasicPhysics. The default omits
    /// the key, so the shipped default stays ubODE. Ferroan has no switch.
    pub basic_physics: bool,
}

impl RunSpec {
    /// True when this grid run serves assets from `ferroan-robust`.
    ///
    /// The caller lives in the Linux harness. Tests compile it too.
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn rust_assets(&self) -> bool {
        self.rust_services.iter().any(|service| service == "assets")
    }

    /// True when this grid run serves user accounts from `ferroan-robust`.
    ///
    /// The caller lives in the Linux harness. Tests compile it too.
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn rust_accounts(&self) -> bool {
        self.rust_services
            .iter()
            .any(|service| service == "accounts")
    }

    /// True when this grid run serves authentication from `ferroan-robust`.
    ///
    /// The caller lives in the Linux harness. Tests compile it too.
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn rust_auth(&self) -> bool {
        self.rust_services.iter().any(|service| service == "auth")
    }
}

pub(crate) fn require_known_services(services: &[String]) -> Result<(), String> {
    for service in services {
        if service != "assets" && service != "accounts" && service != "auth" {
            return Err(format!(
                "{service} is not served by this harness yet; grid --rust wires assets, accounts and auth"
            ));
        }
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Build,
    Up {
        ferroan_assets: bool,
    },
    UpGrid {
        bind: Option<String>,
    },
    PersistentGridCheck,
    Down,
    Run(RunSpec),
    RecordTwice,
    RunningAssets {
        db: Database,
    },
    Compare {
        service: String,
        db: Database,
        backend: AssetBackend,
    },
    Fixture {
        kind: Fixture,
        path: String,
    },
    Diag {
        urls: Vec<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fixture {
    ParcelTiming,
    ObjectCross,
    Caps,
    Login,
    RootArrival,
    RegionParity,
    Circuit,
    Arrival,
    Trailing,
}

pub(crate) enum Outcome {
    Exit(ExitCode),
    Usage,
}

#[derive(Debug)]
enum ParseFail {
    Usage,
    Message(String),
}

pub(crate) fn dispatch(args: &[&str]) -> Outcome {
    let (base, args) = match split_port_base(args) {
        Ok(split) => split,
        Err(text) => {
            eprintln!("live: {text}");
            return Outcome::Exit(ExitCode::from(2));
        }
    };
    match parse(&args) {
        Ok(command) => {
            let persistent = matches!(
                command,
                Command::Up { .. } | Command::RunningAssets { .. } | Command::Down
            );
            if let Err(text) = crate::ports::install(base, persistent) {
                eprintln!("live: {text}");
                return Outcome::Exit(ExitCode::from(2));
            }
            Outcome::Exit(execute(command))
        }
        Err(ParseFail::Usage) => Outcome::Usage,
        Err(ParseFail::Message(text)) => {
            eprintln!("live: {text}");
            Outcome::Exit(ExitCode::from(2))
        }
    }
}

fn split_port_base<'a>(args: &[&'a str]) -> Result<(Option<u16>, Vec<&'a str>), String> {
    let mut base = None;
    let mut kept = Vec::with_capacity(args.len());
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--port-base" {
            let value = args
                .get(index + 1)
                .copied()
                .filter(|value| !value.starts_with('-'))
                .ok_or_else(|| "--port-base needs a port".to_string())?;
            let port: u16 = value
                .parse()
                .map_err(|_| "--port-base needs a port".to_string())?;
            if port == 0 {
                return Err("--port-base needs a port from 1024 up".to_string());
            }
            if base.is_some() {
                return Err("--port-base was given twice".to_string());
            }
            base = Some(port);
            index += 2;
            continue;
        }
        kept.push(args[index]);
        index += 1;
    }
    Ok((base, kept))
}

fn execute(command: Command) -> ExitCode {
    match command {
        Command::Build => live::build(),
        Command::Up { ferroan_assets } => live::up(ferroan_assets),
        Command::UpGrid { bind } => {
            let Some(bind) = bind else {
                return grid::up(Ipv4Addr::LOCALHOST);
            };
            match bind.parse::<Ipv4Addr>() {
                Ok(bind) => grid::up(bind),
                Err(_) => live::finish(Err("--bind requires an IPv4 address".to_string())),
            }
        }
        Command::PersistentGridCheck => crate::grid_smoke::run(),
        Command::Down => live::down(),
        Command::Run(spec) => run_spec(spec),
        Command::RecordTwice => live::record_twice(),
        Command::RunningAssets { db } => match db.require_ready() {
            Ok(()) => live::run_running_assets(),
            Err(error) => live::finish(Err(error)),
        },
        Command::Compare {
            service,
            db,
            backend,
        } => compare_service(&service, db, backend),
        Command::Fixture { kind, path } => fixture(kind, &path),
        Command::Diag { urls } => crate::diag_fetch::run(&urls),
    }
}

fn run_spec(spec: RunSpec) -> ExitCode {
    if let Err(error) = spec.db.require_ready() {
        return live::finish(Err(error));
    }
    if let Err(error) = crate::nodelay::record() {
        return live::finish(Err(error));
    }
    if !spec.grid {
        if spec.ferroan_region {
            return live::finish(Err("--region ferroan is used with grid mode".to_string()));
        }
        if !spec.rust_services.is_empty() {
            return live::finish(Err("--rust is used with grid mode".to_string()));
        }
        return live::run(&spec.scenario, spec.basic_physics);
    }
    if let Err(error) = require_known_services(&spec.rust_services) {
        return live::finish(Err(error));
    }
    grid::run_with(&spec)
}

fn compare_service(service: &str, db: Database, backend: AssetBackend) -> ExitCode {
    if service == "region" {
        return crate::region_report::compare();
    }
    if backend == AssetBackend::Xasset {
        if service != "assets" {
            return live::finish(Err("--backend xasset is used with live assets".to_string()));
        }
        return match db {
            Database::Sqlite => live::finish(Err("XAssetService has no SQLite plugin".to_string())),
            Database::Mysql => crate::xasset_live::compare(crate::xasset_live::XassetDb::Mysql),
            Database::Pgsql => crate::xasset_live::compare(crate::xasset_live::XassetDb::Pgsql),
        };
    }
    if backend == AssetBackend::Fsassets {
        if service != "assets" {
            return live::finish(Err(
                "--backend fsassets is used with live assets".to_string()
            ));
        }
        return match db {
            Database::Sqlite => {
                live::finish(Err("FSAssetService has no SQLite plugin".to_string()))
            }
            Database::Mysql => crate::fsasset_live::compare(crate::fsasset_live::FsassetDb::Mysql),
            Database::Pgsql => crate::fsasset_live::compare(crate::fsasset_live::FsassetDb::Pgsql),
        };
    }
    if let Err(error) = db.require_ready() {
        return live::finish(Err(error));
    }
    match service {
        "assets" => crate::assets::compare(),
        "accounts" => crate::accounts::compare(),
        "auth" => crate::auth::compare(),
        _ => live::finish(Err(format!(
            "live compare serves assets, accounts and auth; {service} is not wired yet"
        ))),
    }
}

fn fixture(kind: Fixture, path: &str) -> ExitCode {
    let path = std::path::Path::new(path);
    match kind {
        Fixture::ParcelTiming => grid::parcel_timing_fixtures(path),
        Fixture::ObjectCross => grid::object_cross_fixtures(path),
        Fixture::Caps => live::finish(crate::caps_fixtures::generate(path)),
        Fixture::Login => grid::login_fixtures(path),
        Fixture::RootArrival => grid::root_arrival_fixtures(path),
        Fixture::RegionParity => grid::parity_fixtures(path),
        Fixture::Circuit => live::finish(crate::circuit_fixtures::generate(path)),
        Fixture::Arrival => live::finish(crate::arrival_fixtures::generate(path)),
        Fixture::Trailing => live::finish(crate::trailing_fixtures::generate(path)),
    }
}

fn parse(args: &[&str]) -> Result<Command, ParseFail> {
    match args {
        ["build"] => Ok(Command::Build),
        ["up"] => Ok(Command::Up {
            ferroan_assets: false,
        }),
        ["up", "--assets", "ferroan"] => Ok(Command::Up {
            ferroan_assets: true,
        }),
        ["up", "grid", rest @ ..] => {
            let flags = flags(rest)?;
            if flags.running
                || flags.rust_services.is_some()
                || flags.db_set
                || flags.backend.is_some()
                || flags.basic_physics
            {
                return Err(ParseFail::Usage);
            }
            if !flags.region_ferroan {
                return Err(ParseFail::Usage);
            }
            Ok(Command::UpGrid { bind: flags.bind })
        }
        ["persistent-grid-check"] => Ok(Command::PersistentGridCheck),
        ["down"] => Ok(Command::Down),
        ["diag", rest @ ..] => Ok(Command::Diag {
            urls: rest.iter().map(|url| (*url).to_owned()).collect(),
        }),
        ["record-twice"] => Ok(Command::RecordTwice),
        ["run", rest @ ..] => parse_run(rest),
        ["compare", service, rest @ ..] => {
            if service.starts_with('-') {
                return Err(ParseFail::Usage);
            }
            let flags = flags(rest)?;
            if flags.running
                || flags.region_ferroan
                || flags.bind.is_some()
                || flags.rust_services.is_some()
                || flags.basic_physics
            {
                return Err(ParseFail::Usage);
            }
            if flags.backend.is_some() && *service != "assets" {
                return Err(ParseFail::Message(
                    "--backend is used with live assets".to_string(),
                ));
            }
            if *service == "region" && (flags.db_set || flags.backend.is_some()) {
                return Err(ParseFail::Usage);
            }
            Ok(Command::Compare {
                service: (*service).to_string(),
                db: flags.db,
                backend: flags.backend.unwrap_or(AssetBackend::Stock),
            })
        }
        ["assets", rest @ ..] => {
            let flags = flags(rest)?;
            if flags.running
                || flags.region_ferroan
                || flags.bind.is_some()
                || flags.rust_services.is_some()
                || flags.basic_physics
            {
                return Err(ParseFail::Usage);
            }
            Ok(Command::Compare {
                service: "assets".to_string(),
                db: flags.db,
                backend: flags.backend.unwrap_or(AssetBackend::Stock),
            })
        }
        ["auth", rest @ ..] => {
            let flags = flags(rest)?;
            if flags.running
                || flags.region_ferroan
                || flags.bind.is_some()
                || flags.rust_services.is_some()
                || flags.backend.is_some()
                || flags.basic_physics
            {
                return Err(if flags.backend.is_some() {
                    ParseFail::Message("--backend is used with live assets".to_string())
                } else {
                    ParseFail::Usage
                });
            }
            Ok(Command::Compare {
                service: "auth".to_string(),
                db: flags.db,
                backend: AssetBackend::Stock,
            })
        }
        ["accounts", rest @ ..] => {
            let flags = flags(rest)?;
            if flags.running
                || flags.region_ferroan
                || flags.bind.is_some()
                || flags.rust_services.is_some()
                || flags.backend.is_some()
                || flags.basic_physics
            {
                return Err(if flags.backend.is_some() {
                    ParseFail::Message("--backend is used with live assets".to_string())
                } else {
                    ParseFail::Usage
                });
            }
            Ok(Command::Compare {
                service: "accounts".to_string(),
                db: flags.db,
                backend: AssetBackend::Stock,
            })
        }
        [name, path] if !name.starts_with('-') && !path.starts_with('-') => {
            let kind = fixture_kind(name).ok_or(ParseFail::Usage)?;
            Ok(Command::Fixture {
                kind,
                path: (*path).to_string(),
            })
        }
        _ => Err(ParseFail::Usage),
    }
}

fn parse_run(args: &[&str]) -> Result<Command, ParseFail> {
    let Some((scenario, rest)) = args.split_first() else {
        return Err(ParseFail::Usage);
    };
    if scenario.starts_with('-') {
        return Err(ParseFail::Usage);
    }
    let (grid, rest) = if rest.first() == Some(&"grid") {
        (true, &rest[1..])
    } else {
        (false, rest)
    };
    let flags = flags(rest)?;
    if flags.bind.is_some() || flags.backend.is_some() {
        return Err(if flags.backend.is_some() {
            ParseFail::Message("--backend is used with live assets".to_string())
        } else {
            ParseFail::Usage
        });
    }
    if flags.running {
        if *scenario != "assets"
            || grid
            || flags.region_ferroan
            || flags.rust_services.is_some()
            || flags.basic_physics
        {
            return Err(ParseFail::Usage);
        }
        return Ok(Command::RunningAssets { db: flags.db });
    }
    if matches!(
        *scenario,
        "prim-capture" | "prim-arrival" | "object-cross" | "child-view" | "parcel-timing"
    ) && (!grid || flags.region_ferroan || !flags.basic_physics)
    {
        return Err(ParseFail::Message(format!(
            "use live run {scenario} grid --physics basicphysics for the Pin8 capture"
        )));
    }
    Ok(Command::Run(RunSpec {
        scenario: (*scenario).to_string(),
        grid,
        ferroan_region: flags.region_ferroan,
        rust_services: flags.rust_services.unwrap_or_default(),
        db: flags.db,
        basic_physics: flags.basic_physics,
    }))
}

struct Flags {
    region_ferroan: bool,
    bind: Option<String>,
    rust_services: Option<Vec<String>>,
    db: Database,
    db_set: bool,
    running: bool,
    backend: Option<AssetBackend>,
    basic_physics: bool,
}

fn flags(args: &[&str]) -> Result<Flags, ParseFail> {
    let mut parsed = Flags {
        region_ferroan: false,
        bind: None,
        rust_services: None,
        db: Database::Sqlite,
        db_set: false,
        running: false,
        backend: None,
        basic_physics: false,
    };
    let mut index = 0;
    while index < args.len() {
        let arg = args[index];
        match arg {
            "--running" => parsed.running = true,
            "--region" => {
                let value = args.get(index + 1).ok_or(ParseFail::Usage)?;
                if *value != "ferroan" {
                    return Err(ParseFail::Usage);
                }
                parsed.region_ferroan = true;
                index += 1;
            }
            "--bind" => {
                let value = args.get(index + 1).ok_or(ParseFail::Usage)?;
                if value.starts_with('-') {
                    return Err(ParseFail::Usage);
                }
                parsed.bind = Some((*value).to_string());
                index += 1;
            }
            "--rust" => {
                let value = args.get(index + 1).ok_or(ParseFail::Usage)?;
                parsed.rust_services = Some(services(value)?);
                index += 1;
            }
            "--db" => {
                let value = args.get(index + 1).ok_or(ParseFail::Usage)?;
                parsed.db = database(value)?;
                parsed.db_set = true;
                index += 1;
            }
            "--backend" => {
                let value = args.get(index + 1).ok_or(ParseFail::Usage)?;
                parsed.backend = Some(asset_backend(value)?);
                index += 1;
            }
            "--physics" => {
                let value = args.get(index + 1).ok_or(ParseFail::Usage)?;
                if *value != "basicphysics" {
                    return Err(ParseFail::Message(
                        "--physics expects basicphysics".to_string(),
                    ));
                }
                parsed.basic_physics = true;
                index += 1;
            }
            _ => return Err(ParseFail::Usage),
        }
        index += 1;
    }
    Ok(parsed)
}

fn services(value: &str) -> Result<Vec<String>, ParseFail> {
    if value.starts_with('-') {
        return Err(ParseFail::Usage);
    }
    let mut names = Vec::new();
    for part in value.split(',') {
        let name = part.trim();
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err(ParseFail::Usage);
        }
        if !names.iter().any(|have: &String| have == name) {
            names.push(name.to_string());
        }
    }
    if names.is_empty() {
        return Err(ParseFail::Usage);
    }
    Ok(names)
}

fn asset_backend(value: &str) -> Result<AssetBackend, ParseFail> {
    match value {
        "xasset" => Ok(AssetBackend::Xasset),
        "fsassets" => Ok(AssetBackend::Fsassets),
        _ => Err(ParseFail::Message(format!(
            "--backend expects xasset or fsassets; {value} is not served by this harness yet"
        ))),
    }
}

fn database(value: &str) -> Result<Database, ParseFail> {
    match value {
        "sqlite" => Ok(Database::Sqlite),
        "mysql" => Ok(Database::Mysql),
        "pgsql" => Ok(Database::Pgsql),
        _ => Err(ParseFail::Message(
            "--db expects sqlite, mysql or pgsql".to_string(),
        )),
    }
}

fn fixture_kind(name: &str) -> Option<Fixture> {
    Some(match name {
        "parcel-timing-fixtures" => Fixture::ParcelTiming,
        "object-cross-fixtures" => Fixture::ObjectCross,
        "caps-fixtures" => Fixture::Caps,
        "login-fixtures" => Fixture::Login,
        "root-arrival-fixtures" => Fixture::RootArrival,
        "region-parity-fixtures" => Fixture::RegionParity,
        "circuit-fixtures" => Fixture::Circuit,
        "arrival-fixtures" => Fixture::Arrival,
        "trailing-fixtures" => Fixture::Trailing,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> RunSpec {
        match parse(args).unwrap() {
            Command::Run(spec) => spec,
            other => panic!("expected a run, got {other:?}"),
        }
    }

    #[test]
    fn grid_rust_assets_parses_in_any_flag_order() {
        let spec = run(&[
            "run", "assets", "grid", "--db", "sqlite", "--rust", "assets", "--region", "ferroan",
        ]);
        assert_eq!(spec.scenario, "assets");
        assert!(spec.grid);
        assert!(spec.ferroan_region);
        assert_eq!(spec.rust_services, vec!["assets".to_string()]);
        assert_eq!(spec.db, Database::Sqlite);
        assert!(spec.rust_assets());
        assert!(!spec.rust_accounts());
        assert!(!spec.rust_auth());
    }

    #[test]
    fn grid_rust_accounts_and_the_pair_parse() {
        let accounts = run(&["run", "login", "grid", "--rust", "accounts"]);
        assert!(accounts.rust_accounts());
        assert!(!accounts.rust_assets());
        let both = run(&["run", "login", "grid", "--rust", "assets,accounts"]);
        assert!(both.rust_assets());
        assert!(both.rust_accounts());
        assert!(!both.rust_auth());
        let auth = run(&["run", "login", "grid", "--rust", "auth"]);
        assert!(auth.rust_auth());
        assert!(!auth.rust_accounts());
        let all = run(&["run", "login", "grid", "--rust", "assets,accounts,auth"]);
        assert!(all.rust_assets() && all.rust_accounts() && all.rust_auth());
        assert!(require_known_services(&both.rust_services).is_ok());
        assert!(require_known_services(&all.rust_services).is_ok());
    }

    #[test]
    fn rust_list_trims_and_drops_duplicates() {
        let spec = run(&["run", "all", "grid", "--rust", "assets, assets"]);
        assert_eq!(spec.rust_services, vec!["assets".to_string()]);
        assert!(!spec.ferroan_region);
        assert_eq!(spec.db, Database::Sqlite);
    }

    #[test]
    fn compare_assets_defaults_to_sqlite() {
        match parse(&["compare", "assets"]).unwrap() {
            Command::Compare {
                service,
                db,
                backend,
            } => {
                assert_eq!(service, "assets");
                assert_eq!(db, Database::Sqlite);
                assert_eq!(backend, AssetBackend::Stock);
            }
            other => panic!("expected compare, got {other:?}"),
        }
        match parse(&["assets", "--db", "pgsql"]).unwrap() {
            Command::Compare {
                service,
                db,
                backend,
            } => {
                assert_eq!(service, "assets");
                assert_eq!(db, Database::Pgsql);
                assert_eq!(backend, AssetBackend::Stock);
            }
            other => panic!("expected compare, got {other:?}"),
        }
        match parse(&["accounts"]).unwrap() {
            Command::Compare {
                service,
                db,
                backend,
            } => {
                assert_eq!(service, "accounts");
                assert_eq!(db, Database::Sqlite);
                assert_eq!(backend, AssetBackend::Stock);
            }
            other => panic!("expected compare, got {other:?}"),
        }
        match parse(&["auth"]).unwrap() {
            Command::Compare { service, db, .. } => {
                assert_eq!(service, "auth");
                assert_eq!(db, Database::Sqlite);
            }
            other => panic!("expected compare, got {other:?}"),
        }
        assert!(require_known_services(&["accounts".into()]).is_ok());
    }

    #[test]
    fn basicphysics_is_opt_in_on_a_run() {
        let spec = run(&["run", "walk", "--physics", "basicphysics"]);
        assert!(spec.basic_physics);
        assert!(!spec.grid);
        let grid = run(&[
            "run",
            "walk",
            "grid",
            "--region",
            "ferroan",
            "--physics",
            "basicphysics",
        ]);
        assert!(grid.basic_physics);
        assert!(grid.ferroan_region);
        assert!(!run(&["run", "login", "grid"]).basic_physics);
        assert!(parse(&["run", "walk", "--physics", "ubode"]).is_err());
        assert!(parse(&["compare", "region", "--physics", "basicphysics"]).is_err());
        assert!(
            parse(&[
                "up",
                "grid",
                "--region",
                "ferroan",
                "--physics",
                "basicphysics"
            ])
            .is_err()
        );
    }

    #[test]
    fn prim_capture_requires_the_csharp_grid_and_basicphysics() {
        for scenario in [
            "prim-capture",
            "prim-arrival",
            "object-cross",
            "child-view",
            "parcel-timing",
        ] {
            assert!(parse(&["run", scenario, "grid"]).is_err());
            assert!(parse(&["run", scenario, "grid", "--physics", "ubode"]).is_err());
            assert!(parse(&["run", scenario, "--physics", "basicphysics"]).is_err());
            assert!(
                parse(&[
                    "run",
                    scenario,
                    "grid",
                    "--region",
                    "ferroan",
                    "--physics",
                    "basicphysics"
                ])
                .is_err()
            );
            assert!(run(&["run", scenario, "grid", "--physics", "basicphysics"]).basic_physics);
        }
    }

    #[test]
    fn compare_region_takes_no_database() {
        match parse(&["compare", "region"]).unwrap() {
            Command::Compare {
                service,
                db,
                backend,
            } => {
                assert_eq!(service, "region");
                assert_eq!(db, Database::Sqlite);
                assert_eq!(backend, AssetBackend::Stock);
            }
            other => panic!("expected compare, got {other:?}"),
        }
        assert!(parse(&["compare", "region", "--db", "mysql"]).is_err());
        assert!(parse(&["compare", "region", "--backend", "xasset"]).is_err());
    }

    #[test]
    fn existing_live_forms_still_parse() {
        assert!(matches!(parse(&["build"]), Ok(Command::Build)));
        assert!(matches!(parse(&["record-twice"]), Ok(Command::RecordTwice)));
        assert!(parse(&["record-twice", "login"]).is_err());
        assert!(matches!(
            parse(&["up"]),
            Ok(Command::Up {
                ferroan_assets: false
            })
        ));
        assert!(matches!(
            parse(&["up", "--assets", "ferroan"]),
            Ok(Command::Up {
                ferroan_assets: true
            })
        ));
        assert!(matches!(
            parse(&["up", "grid", "--region", "ferroan"]),
            Ok(Command::UpGrid { bind: None })
        ));
        assert!(matches!(
            parse(&[
                "up",
                "grid",
                "--bind",
                "192.0.2.1",
                "--region",
                "ferroan"
            ]),
            Ok(Command::UpGrid { bind: Some(bind) }) if bind == "192.0.2.1"
        ));
        assert!(matches!(parse(&["down"]), Ok(Command::Down)));
        assert!(matches!(
            parse(&["persistent-grid-check"]),
            Ok(Command::PersistentGridCheck)
        ));
        let (base, kept) =
            split_port_base(&["run", "login", "grid", "--port-base", "20000"]).unwrap();
        assert_eq!(base, Some(20_000));
        assert_eq!(kept, vec!["run", "login", "grid"]);
        let spec = run(&kept);
        assert_eq!(spec.scenario, "login");
        assert!(spec.grid);
        assert!(split_port_base(&["run", "--port-base"]).is_err());
        assert!(split_port_base(&["run", "--port-base", "nope"]).is_err());
        assert!(split_port_base(&["run", "--port-base", "0"]).is_err());
        assert!(split_port_base(&["--port-base", "20000", "--port-base", "21000"]).is_err());
        let spec = run(&["run", "login"]);
        assert!(!spec.grid);
        assert_eq!(spec.scenario, "login");
        let spec = run(&["run", "all", "grid"]);
        assert!(spec.grid);
        assert!(spec.rust_services.is_empty());
        let spec = run(&["run", "caps", "grid", "--region", "ferroan"]);
        assert!(spec.ferroan_region);
        assert!(matches!(
            parse(&["run", "assets", "--running"]),
            Ok(Command::RunningAssets {
                db: Database::Sqlite
            })
        ));
        assert!(matches!(
            parse(&["login-fixtures", "/tmp/capture"]),
            Ok(Command::Fixture {
                kind: Fixture::Login,
                path
            }) if path == "/tmp/capture"
        ));
    }

    #[test]
    fn incomplete_flags_are_usage() {
        assert!(parse(&["run"]).is_err());
        assert!(parse(&["run", "all", "grid", "--rust"]).is_err());
        assert!(parse(&["run", "all", "grid", "--rust", "assets,"]).is_err());
        assert!(parse(&["run", "all", "grid", "--region"]).is_err());
        assert!(parse(&["up", "grid"]).is_err());
        assert!(parse(&["compare"]).is_err());
        assert!(parse(&["assets", "grid"]).is_err());
    }

    #[test]
    fn mysql_and_pgsql_name_p1a1_and_unknown_services_are_refused() {
        assert_eq!(
            Database::Mysql.require_ready().unwrap_err(),
            "mysql needs P1.A1 (#59) before the harness can use it"
        );
        assert_eq!(
            Database::Pgsql.require_ready().unwrap_err(),
            "pgsql needs P1.A1 (#59) before the harness can use it"
        );
        assert!(Database::Sqlite.require_ready().is_ok());
        match parse(&["assets", "--backend", "xasset", "--db", "mysql"]).unwrap() {
            Command::Compare {
                service,
                db,
                backend,
            } => {
                assert_eq!(service, "assets");
                assert_eq!(db, Database::Mysql);
                assert_eq!(backend, AssetBackend::Xasset);
            }
            other => panic!("expected compare, got {other:?}"),
        }
        match parse(&["compare", "assets", "--db", "pgsql", "--backend", "xasset"]).unwrap() {
            Command::Compare { db, backend, .. } => {
                assert_eq!(db, Database::Pgsql);
                assert_eq!(backend, AssetBackend::Xasset);
            }
            other => panic!("expected compare, got {other:?}"),
        }
        match parse(&["assets", "--db", "mysql"]).unwrap() {
            Command::Compare { db, backend, .. } => {
                assert_eq!(backend, AssetBackend::Stock);
                assert_eq!(
                    db.require_ready().unwrap_err(),
                    "mysql needs P1.A1 (#59) before the harness can use it"
                );
            }
            other => panic!("expected compare, got {other:?}"),
        }
        match parse(&["assets", "--backend", "fsassets", "--db", "mysql"]).unwrap() {
            Command::Compare { db, backend, .. } => {
                assert_eq!(db, Database::Mysql);
                assert_eq!(backend, AssetBackend::Fsassets);
            }
            other => panic!("expected compare, got {other:?}"),
        }
        match parse(&["assets", "--backend", "fsassets", "--db", "pgsql"]).unwrap() {
            Command::Compare { db, backend, .. } => {
                assert_eq!(db, Database::Pgsql);
                assert_eq!(backend, AssetBackend::Fsassets);
            }
            other => panic!("expected compare, got {other:?}"),
        }
        let refused = parse(&["assets", "--backend", "legacy"]).unwrap_err();
        assert!(matches!(refused, ParseFail::Message(text) if text.contains("fsassets")));
        assert!(matches!(
            parse(&["accounts", "--backend", "xasset"]),
            Err(ParseFail::Message(text)) if text.contains("live assets")
        ));
        assert!(matches!(
            parse(&["run", "assets", "grid", "--backend", "xasset", "--db", "mysql"]),
            Err(ParseFail::Message(text)) if text.contains("live assets")
        ));
        let error = require_known_services(&["assets".into(), "inventory".into()]).unwrap_err();
        assert!(error.contains("inventory"));
        assert!(error.contains("assets"));
    }
}
