// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Repository tasks, run as `cargo xtask <task>`.
//!
//! `check` verifies that every Markdown link to a local file resolves and that
//! every limit id is listed in `docs/LIMITS.md` and named from a call site,
//! then runs formatting, clippy (warnings denied) and the workspace tests.
//! With `--docs-only` it stops after the Markdown and limit-id checks.
//!
//! `oracle` builds and runs the C# reference oracle: `build`, `generate` and
//! `verify`. The Linux live workflow runs `verify` against pinned references.
//!
//! `schema` builds the C# schema oracle and generates or verifies the SQLite
//! fixtures. The Linux live workflow runs `verify` with the .NET SDK and
//! reference checkouts.
//!
//! `config` builds the C# config oracle and generates or verifies the resolved
//! INI fixtures. The Linux live workflow runs `verify` with the .NET SDK and
//! reference checkouts.
//!
//! `http` builds the C# ServerUtils oracle and generates or verifies the
//! query-string and Robust XML fixtures. `http status-verify` runs the pinned
//! Grid, Presence and GridUser connectors against the status fixtures. Both
//! need the .NET SDK and the reference checkouts. The Linux live workflow
//! runs `verify` and `status-verify`.
//!
//! `throttle generate` and `throttle verify` read the pinned category token
//! buckets and compare `fixtures/throttle/rates.json`. Neither starts a region.
//!
//! `presence generate` and `presence verify` record and compare the pinned Null
//! provider's calls and results in `fixtures/presence/null.json`.
//! `presence cross postgres` cross-reads the PostgreSQL v4 store with Pin8.
//!
//! `regionstore generate` builds the region-store oracle and writes the SQLite
//! object fixture. `regionstore settings` writes the region-settings fixture.
//! `regionstore serialize` writes XML2 and original XML for that object fixture.
//! `regionstore read` reads that XML back through the pin and writes the fields.
//! `regionstore verify` reloads the object fixture. `regionstore cross`
//! reloads SQLite and throwaway MySQL and PostgreSQL containers, then removes
//! the containers. None of them starts a server or the live harness.
//!
//! `asset` builds the C# asset oracle and generates or verifies `AssetBase`
//! XML fixtures. It needs the .NET SDK and the reference checkouts. The Linux
//! live workflow runs `verify`. `asset xasset-cross` runs the XAsset plugins
//! against the local schema-server containers. `asset fsassets-cross` runs the
//! FSAssets plugins the same way. `asset fsassets-files` compares FSAssets
//! paths and decompressed bytes. `griduser cross` runs the GridUser plugins
//! the same way. `griduser service` runs one `POST /griduser` sequence on a
//! new SQLite file. `avatar cross` runs the avatar plugins the same way. None
//! of these starts a server or the live harness.
//!
//! `access` builds the C# full-region oracle and generates or verifies
//! `fixtures/access`. It calls `Scene.CheckLandPositionAccess` with
//! `DefaultPermissionsModule`. The Linux live workflow runs `verify`. It does
//! not start a region.
//!
//! `live` builds the pinned Tranquillity region into `.scratch/live/`,
//! starts or stops it, and runs bot scenarios against it. It runs on Linux.
//! The live CI job runs standalone and grid mode.
//!
//! `docker build` builds the Ferroan and pinned Tranquillity images without
//! pushing them. `docker smoke` logs in through that grid. Both run on Linux.
//!
//! `ci scope` classifies a pull request as documentation-only. `ci gate` is
//! the required live check: every parallel live job succeeded, or, on a
//! documentation-only pull request, every one was skipped. `ci ensure-sqlite`
//! installs the `sqlite3` command with `apt-get` only when it is missing.
//! `ci ensure-db-clients` does the same for the `mysql` and `psql` commands.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod access_oracle;
mod accounts;
mod arrival_fixtures;
mod asset_oracle;
mod assets;
mod auth;
mod caps_fixtures;
mod ci;
mod circuit_fixtures;
mod config_oracle;
mod console_client;
mod console_forms;
mod diag_fetch;
mod docker;
mod dotnet_ci;
mod ferrobot;
mod ferrobot_build;
mod fixture_oracle;
mod fsasset_live;
mod grid;
mod grid_smoke;
mod grid_state;
mod http_oracle;
mod inventory_probe;
mod limit_ids;
mod limits_probe;
mod live;
mod live_args;
mod live_proxy;
mod login_gate;
mod nodelay;
mod oracle;
mod oracle_reference;
mod pin3_oracle;
mod ports;
mod region_report;
mod regionstore;
mod schema;
mod signal;
mod trailing_fixtures;
mod twice;
mod xasset_live;

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode};

/// Directories never searched for Markdown files.
const SKIP_DIRS: &[&str] = &[".git", "target", ".scratch"];

const USAGE: &str = "\
usage: cargo xtask check [--docs-only]
       cargo xtask oracle build
       cargo xtask oracle generate [fixture-dir]
       cargo xtask oracle verify [fixture-dir]
       cargo xtask schema generate [directory]
       cargo xtask schema verify [directory]
       cargo xtask schema servers
       cargo xtask config generate [directory]
       cargo xtask config verify [directory]
       cargo xtask http generate [directory]
       cargo xtask http verify [directory]
       cargo xtask http auth-generate
       cargo xtask http auth-verify
       cargo xtask http status-generate
       cargo xtask http status-verify
       cargo xtask throttle generate
       cargo xtask throttle verify
       cargo xtask presence cross postgres
       cargo xtask presence generate
       cargo xtask presence verify
       cargo xtask asset generate [directory]
       cargo xtask asset verify [directory]
       cargo xtask asset cross
       cargo xtask asset xasset-cross
       cargo xtask asset fsassets-cross
       cargo xtask asset fsassets-files
       cargo xtask griduser cross
       cargo xtask griduser service
       cargo xtask avatar service-generate|service-verify
       cargo xtask avatar appearance-generate|appearance-verify
       cargo xtask avatar cross
       cargo xtask regionstore generate
       cargo xtask regionstore prim-seed
       cargo xtask regionstore settings
       cargo xtask regionstore coalesced
       cargo xtask regionstore parcels
       cargo xtask regionstore serialize
       cargo xtask regionstore read
       cargo xtask regionstore verify
       cargo xtask regionstore cross
       cargo xtask regionstore bulk <db> <region-uuid> <count>
       cargo xtask pin3 generate|verify
       cargo xtask access generate|verify
       cargo xtask permissions generate|verify
       cargo xtask live build
       FERROAN_LIVE_PORT_BASE or --port-base <port> (a multiple of 32 outside the ephemeral range) moves every live port and gives the run its own directory
       cargo xtask live record-twice
       cargo xtask live up [--assets ferroan]
       cargo xtask live up grid --region ferroan [--bind <IPv4>]
       cargo xtask live persistent-grid-check
       cargo xtask live diag [url...]
       cargo xtask live down
       cargo xtask live run object-cross grid --physics basicphysics
       cargo xtask live object-cross-fixtures <capture-dir>
       cargo xtask live run parcel-timing grid --physics basicphysics
       cargo xtask live parcel-timing-fixtures <capture-dir>
       cargo xtask live run parcel-empty|parcel-multi|parcel-lone|parcel-legacy grid --physics basicphysics
       cargo xtask live run prim-capture grid --physics basicphysics
       cargo xtask live run prim-arrival grid --physics basicphysics
       cargo xtask live run <scenario>|all [grid] [--region ferroan] [--physics basicphysics] [--rust <service>,…] [--db sqlite|mysql|pgsql]
       cargo xtask live run login grid --region ferroan
       cargo xtask live run limits grid --region ferroan
       cargo xtask live run caps grid [--region ferroan]
       cargo xtask live run ferrobot [grid [--region ferroan]]
       cargo xtask live run assets grid --region ferroan
       cargo xtask live run <scenario>|all grid --rust assets [--db sqlite]
       cargo xtask live run login grid --rust accounts
       cargo xtask live run login grid --rust auth
       cargo xtask live run login grid --rust assets,accounts
       cargo xtask live caps-fixtures <capture-dir>
       cargo xtask live login-fixtures <capture-dir>
       cargo xtask live region-parity-fixtures <capture-dir>
       cargo xtask live root-arrival-fixtures <capture-dir>
       cargo xtask live circuit-fixtures <udp-capture>
       cargo xtask live arrival-fixtures <udp-capture>
       cargo xtask live trailing-fixtures <udp-capture>
       cargo xtask live run assets --running
       cargo xtask live compare <service> [--db sqlite|mysql|pgsql]
       cargo xtask live compare region
       cargo xtask live assets [--db sqlite|mysql|pgsql]
       cargo xtask live assets --backend xasset --db mysql|pgsql
       cargo xtask live accounts [--db sqlite|mysql|pgsql]
       cargo xtask live auth [--db sqlite|mysql|pgsql]
       cargo xtask docker build
       cargo xtask docker smoke
       cargo xtask console-client
       cargo xtask console-forms
       cargo xtask ci scope
       cargo xtask ci gate
       cargo xtask ci ensure-sqlite
       cargo xtask ci ensure-db-clients";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    if refs.first() == Some(&"live") {
        return match live_args::dispatch(&refs[1..]) {
            live_args::Outcome::Exit(code) => code,
            live_args::Outcome::Usage => {
                eprintln!("{USAGE}");
                ExitCode::from(2)
            }
        };
    }
    // Docker smoke and console-forms open listeners too. `live` installs from
    // the flag or the environment inside dispatch; every other command reads
    // the environment here.
    if let Err(text) = ports::install(None, false) {
        eprintln!("xtask: {text}");
        return ExitCode::from(2);
    }
    match refs.as_slice() {
        ["check"] => check(false),
        ["pin3", "generate"] => pin3_oracle::run(true),
        ["pin3", "verify"] => pin3_oracle::run(false),
        ["access", "generate"] => access_oracle::run("access", "AccessOracle", true),
        ["access", "verify"] => access_oracle::run("access", "AccessOracle", false),
        ["permissions", "generate"] => access_oracle::run("permissions", "PermissionsOracle", true),
        ["permissions", "verify"] => access_oracle::run("permissions", "PermissionsOracle", false),
        ["check", "--docs-only"] => check(true),
        ["oracle", "build"] => oracle::run(oracle::CommandKind::Build, None),
        ["oracle", "generate"] => oracle::run(oracle::CommandKind::Generate, None),
        ["oracle", "verify"] => oracle::run(oracle::CommandKind::Verify, None),
        ["oracle", "generate", dir] => {
            oracle::run(oracle::CommandKind::Generate, Some(PathBuf::from(*dir)))
        }
        ["oracle", "verify", dir] => {
            oracle::run(oracle::CommandKind::Verify, Some(PathBuf::from(*dir)))
        }
        ["schema", "generate"] => schema::generate(None),
        ["schema", "generate", directory] => schema::generate(Some(PathBuf::from(directory))),
        ["schema", "verify"] => schema::verify(None),
        ["schema", "verify", directory] => schema::verify(Some(PathBuf::from(directory))),
        ["schema", "servers"] => schema::servers(),
        ["config", "generate"] => config_oracle::generate(None),
        ["config", "generate", directory] => {
            config_oracle::generate(Some(PathBuf::from(directory)))
        }
        ["config", "verify"] => config_oracle::verify(None),
        ["config", "verify", directory] => config_oracle::verify(Some(PathBuf::from(directory))),
        ["http", "generate"] => http_oracle::generate(None),
        ["http", "generate", directory] => http_oracle::generate(Some(PathBuf::from(directory))),
        ["http", "verify"] => http_oracle::verify(None),
        ["http", "verify", directory] => http_oracle::verify(Some(PathBuf::from(directory))),
        ["http", "auth-generate"] => http_oracle::auth_generate(),
        ["http", "auth-verify"] => http_oracle::auth_verify(),
        ["http", "status-generate"] => http_oracle::status_generate(),
        ["http", "status-verify"] => http_oracle::status_verify(),
        ["throttle", "generate"] => fixture_oracle::run("throttle", "ThrottleOracle.csproj", true),
        ["throttle", "verify"] => fixture_oracle::run("throttle", "ThrottleOracle.csproj", false),
        ["presence", "generate"] => fixture_oracle::run("presence", "PresenceOracle.csproj", true),
        ["presence", "cross", "postgres"] => asset_oracle::presence_postgres_cross(),
        ["presence", "verify"] => fixture_oracle::run("presence", "PresenceOracle.csproj", false),
        ["asset", "generate"] => asset_oracle::generate(None),
        ["asset", "generate", directory] => asset_oracle::generate(Some(PathBuf::from(directory))),
        ["asset", "verify"] => asset_oracle::verify(None),
        ["asset", "verify", directory] => asset_oracle::verify(Some(PathBuf::from(directory))),
        ["asset", "cross"] => asset_oracle::cross(),
        ["asset", "xasset-cross"] => asset_oracle::xasset_cross(),
        ["asset", "fsassets-cross"] => asset_oracle::fsassets_cross(),
        ["asset", "fsassets-files"] => asset_oracle::fsassets_files(),
        ["griduser", "cross"] => asset_oracle::griduser_cross(),
        ["griduser", "service"] => asset_oracle::griduser_service(),
        ["avatar", "appearance-generate"] => asset_oracle::avatar_appearance(true),
        ["avatar", "appearance-verify"] => asset_oracle::avatar_appearance(false),
        ["avatar", "service-generate"] => asset_oracle::avatar_service(true),
        ["avatar", "service-verify"] => asset_oracle::avatar_service(false),
        ["avatar", "cross"] => asset_oracle::avatar_cross(),
        ["regionstore", "prim-seed"] => regionstore::prim_seed(),
        ["regionstore", "generate"] => regionstore::generate(),
        ["regionstore", "parcels"] => regionstore::parcels(),
        ["regionstore", "settings"] => regionstore::settings(),
        ["regionstore", "coalesced"] => regionstore::coalesced(),
        ["regionstore", "serialize"] => regionstore::serialize(),
        ["regionstore", "read"] => regionstore::read(),
        ["regionstore", "verify"] => regionstore::verify(),
        ["regionstore", "cross"] => regionstore::cross(),
        ["regionstore", "bulk", db, region, count] => regionstore::bulk(db, region, count),
        ["docker", "build"] => docker::build(),
        ["docker", "smoke"] => docker::smoke(),
        ["console-client"] => console_client::run(),
        ["console-forms"] => console_forms::run(),
        ["ci", "scope"] => ci::scope(),
        ["ci", "gate"] => ci::gate(),
        ["ci", "ensure-sqlite"] => ci::ensure_sqlite(),
        ["ci", "ensure-db-clients"] => ci::ensure_db_clients(),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn check(docs_only: bool) -> ExitCode {
    let root = repo_root();
    if !check_markdown(&root) || !limit_ids::check(&root) || !login_gate::check(&root) {
        return ExitCode::FAILURE;
    }
    if docs_only {
        return ExitCode::SUCCESS;
    }
    let steps: [&[&str]; 3] = [
        &["fmt", "--all", "--", "--check"],
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
        &["test", "--workspace", "--locked"],
    ];
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    for step in steps {
        println!("$ cargo {}", step.join(" "));
        match Command::new(&cargo).args(step).current_dir(&root).status() {
            Ok(status) if status.success() => {}
            Ok(_) => return ExitCode::FAILURE,
            Err(error) => {
                eprintln!("could not run cargo: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

/// The workspace root: the parent of this crate's manifest directory.
pub(crate) fn repo_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().map(Path::to_path_buf).unwrap_or(manifest)
}

fn check_markdown(root: &Path) -> bool {
    let mut files = Vec::new();
    collect_markdown(root, &mut files);
    files.sort();
    let mut problems = Vec::new();
    for file in &files {
        match fs::read_to_string(file) {
            Ok(text) => problems.extend(broken_links(root, file, &text)),
            Err(error) => problems.push(format!("{}: unreadable: {error}", display(root, file))),
        }
    }
    for problem in &problems {
        eprintln!("{problem}");
    }
    if problems.is_empty() {
        println!("Markdown: local links resolve in {} files.", files.len());
    }
    problems.is_empty()
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if !SKIP_DIRS.iter().any(|skip| name == *skip) {
                collect_markdown(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
}

/// Reports links in `text` whose local target does not exist inside `root`.
fn broken_links(root: &Path, file: &Path, text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut fenced = false;
    let base = file.parent().unwrap_or(root);
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        for target in link_targets(line) {
            let Some(path) = local_path(target) else {
                continue;
            };
            let resolved = normalize(&base.join(path));
            if !resolved.starts_with(root) || !resolved.exists() {
                problems.push(format!(
                    "{}:{}: missing local target {target}",
                    display(root, file),
                    index + 1
                ));
            }
        }
    }
    problems
}

/// The targets of inline links `[text](target)` on one line.
fn link_targets(line: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("](") {
        let after = &rest[start + 2..];
        let end = after
            .find(|c: char| c == ')' || c.is_whitespace())
            .unwrap_or(after.len());
        let target = after[..end].trim_start_matches('<').trim_end_matches('>');
        if !target.is_empty() {
            targets.push(target);
        }
        rest = &after[end..];
    }
    targets
}

/// The decoded file path of a link, or `None` for remote URLs and in-page anchors.
fn local_path(target: &str) -> Option<String> {
    if target.contains("://") || target.starts_with("mailto:") || target.starts_with('#') {
        return None;
    }
    let path = target.split(['#', '?']).next().unwrap_or("");
    if path.is_empty() {
        return None;
    }
    Some(percent_decode(path))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(high * 16 + low);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Resolves `.` and `..` without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn display(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_inline_link_targets() {
        let line = "See [a](docs/A.md), [b](<B.md>) and [c](https://example.com).";
        assert_eq!(
            link_targets(line),
            vec!["docs/A.md", "B.md", "https://example.com"]
        );
    }

    #[test]
    fn skips_remote_links_and_anchors() {
        assert_eq!(local_path("https://example.com/x.md"), None);
        assert_eq!(local_path("#section"), None);
        assert_eq!(local_path("docs/A.md#part").as_deref(), Some("docs/A.md"));
    }

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(percent_decode("My%20File.md"), "My File.md");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("bad%zzescape"), "bad%zzescape");
    }

    #[test]
    fn normalizes_parent_components() {
        assert_eq!(
            normalize(Path::new("/r/docs/../README.md")),
            PathBuf::from("/r/README.md")
        );
    }

    #[test]
    fn reports_missing_targets_outside_fences() {
        let root = repo_root();
        let file = root.join("README.md");
        let text = "[ok](LICENSE)\n[gone](no-such-file.md)\n```\n[ignored](missing.md)\n```\n";
        let problems = broken_links(&root, &file, text);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("no-such-file.md"));
    }

    #[test]
    fn required_check_names_stay_put() {
        let root = repo_root();
        let live = fs::read_to_string(root.join(".github/workflows/live.yml")).unwrap();
        let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
        assert_eq!(live.matches("name: live (ubuntu-latest)").count(), 1);
        assert!(ci.contains("name: checks (${{ matrix.os }})"));
        // Linux is its own job so the service containers stay off Windows and
        // macOS. The job name is still the required check `checks (ubuntu-latest)`.
        assert_eq!(ci.matches("name: checks (ubuntu-latest)").count(), 1);
        assert!(ci.contains("os: [windows-latest, macos-latest]"));
        assert!(!ci.contains("ubuntu-latest, windows-latest, macos-latest"));
        assert_eq!(live.matches("cargo xtask docker smoke").count(), 2);
        assert_eq!(
            live.matches("name: Build images and smoke the example grid")
                .count(),
            1
        );
        assert!(live.contains("cargo xtask ci scope"));
        assert!(live.contains("cargo xtask ci gate"));
        assert!(ci.contains("cargo xtask ci scope"));
        assert!(!live.contains("tools/ci"));
        assert!(!ci.contains("tools/ci"));
        // Standalone Ferrobot does not publish the region. The job builds it.
        let ferrobot = live
            .split("name: live ferrobot")
            .nth(1)
            .unwrap()
            .split("name: live services")
            .next()
            .unwrap();
        assert!(ferrobot.contains("cargo xtask live build"));
        let images = live
            .split("name: live images")
            .nth(1)
            .unwrap()
            .split("name: live (ubuntu-latest)")
            .next()
            .unwrap();
        assert!(images.contains("vars.FERROAN_SELF_HOSTED == 'true'"));
        assert!(images.contains("github.event_name != 'pull_request'"));
        assert!(
            images.contains("github.event.pull_request.head.repo.full_name == github.repository")
        );
        assert!(images.contains("fromJSON('[\"self-hosted\",\"linux\",\"x64\",\"ferroan\"]')"));
        assert!(images.contains("|| 'ubuntu-latest'"));
        assert!(!images.contains("concurrency:"));
        assert!(!images.contains("cancel-in-progress"));
        assert!(images.contains("flock -o"));
        assert!(images.contains("${RUNNER_TEMP}/../ferroan-live-images.lock"));
        assert!(!images.contains("/tmp/ferroan-live-images.lock"));
        assert!(images.contains("RUNNER_ENVIRONMENT"));
        assert!(images.contains("runner.environment == 'self-hosted'"));
        assert!(images.contains("runner.name"));
        assert!(images.contains("rust-live-self-hosted-"));
        assert!(images.contains("rust-save:"));
        assert!(images.contains("refs/heads/main"));
        assert!(images.contains("github.event_name == 'schedule'"));
        let gate = live.split("name: live (ubuntu-latest)").nth(1).unwrap();
        assert!(gate.contains("runs-on: ubuntu-latest"));
        assert!(!gate.contains("self-hosted"));
        assert!(!gate.contains("FERROAN_SELF_HOSTED"));
        let setup = fs::read_to_string(root.join(".github/actions/live-setup/action.yml")).unwrap();
        assert!(setup.contains("cargo xtask ci ensure-sqlite"));
        assert!(!setup.contains("apt-get"));
        assert!(!live.contains("apt-get"));
    }
}
