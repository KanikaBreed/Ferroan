# Testing

## The local check

```sh
cargo xtask check              # links, formatting, clippy, tests
cargo xtask check --docs-only  # Markdown links only
```

The command is the same on every platform. Workspace Clippy lints deny `unwrap_used`, `expect_used` and `panic` outside tests. A test crate may allow them. An internal invariant that stays has a narrow `#[allow]` and an `// INFALLIBLE:` comment. [Ordinary CI](../.github/workflows/ci.yml) runs it on Linux, Windows and macOS, using committed fixtures without the C# references or .NET SDK. The separate [Linux live workflow](../.github/workflows/live.yml) installs .NET, checks out the pinned references, verifies the C# oracles and runs real servers. It does not rerun the whole Rust test suite with those references present.

## The Rust toolchain

[`rust-toolchain.toml`](../rust-toolchain.toml) pins the compiler. `cargo` in this repository uses that file, which also selects the `minimal` profile and the `clippy` and `rustfmt` components. Do not change the machine's default with `rustup default` or a directory override.

The "Set up Rust" step in ordinary CI and the live workflow prints `rustup show`, `rustc --version` and `cargo clippy --version`. Those lines are the evidence that the pin is the compiler that ran.

To move the pin:

1. On a branch, set `channel` to `stable` and push. Do not merge that revision.
2. Read `rustc --version` and `cargo clippy --version` from "Set up Rust" on Linux, Windows and macOS.
3. Set `channel` to the `1.x.y` triple those lines name. If the three platforms disagree, pin the newest triple a runner printed, record every line, and say why. Keep `profile = "minimal"` and the `clippy` and `rustfmt` components.
4. `cargo xtask check` on Linux, Windows and macOS must print that compiler. Record the lines in the task row.

The current pin is `1.100.0`. On 19 November 2026, with `channel` still `stable`, Linux and Windows printed `rustc 1.99.1` and macOS printed `rustc 1.100.0`. The file makes all three install `1.100.0`.

## The C# oracle

`cargo xtask oracle build`, `generate` and `verify` build [tools/reference-oracle](../tools/reference-oracle/README.md) against the pinned libopenmetaverse checkout and read or write [fixtures/oracle](../fixtures/oracle/README.md). `FERROAN_REF` selects the checkout root and defaults to `~/Git/reference`. The fixtures are committed. Packet fixtures are one datagram per message in the template; the layout is in [PROTOCOL.md](PROTOCOL.md). The Linux live workflow runs `cargo xtask oracle verify` with .NET SDK 10. The reference checkout is only read; build output goes under `.scratch/`.

## Schema fixtures

`cargo xtask schema generate` and `verify` build [tools/schema-oracle](../tools/schema-oracle/README.md) and run Tranquillity's `Migration.Update` on SQLite. The databases are committed under [fixtures/db/sqlite](../fixtures/db/sqlite/README.md). The Linux live workflow runs `schema verify`; ordinary CI opens the committed databases in Rust tests. MySQL and PostgreSQL are checked by the same Rust code when `FERROAN_MYSQL_URL` or `FERROAN_POSTGRES_URL` is set; neither current workflow supplies those servers.

## Configuration fixtures

`cargo xtask config generate` and `verify` build [tools/config-oracle](../tools/config-oracle/README.md) and resolve INI files with Nini. The dumps are committed under [fixtures/config](../fixtures/config/README.md). The Linux live workflow runs `config verify`, including the shipped INI cases. The Rust tests always check the small cases. The two Rust cases that open Tranquillity's shipped INI files run when the reference checkout is present and are skipped otherwise, including in ordinary CI.

## HTTP fixtures

`cargo xtask http generate` and `verify` build [tools/http-oracle](../tools/http-oracle/README.md) and call Tranquillity's query-string and XML helpers. The committed `fixtures/http` files are read by `ferroan-http` tests on all three CI platforms; the Linux live workflow runs `http verify` against the pinned C# source. `cargo xtask http status-verify` runs the same live job against the 36 connector status fixtures in [fixtures/http-status](../fixtures/http-status/README.md).

## Asset fixtures

`cargo xtask asset generate` and `verify` build [tools/asset-oracle](../tools/asset-oracle/README.md) and serialize `AssetBase` with Tranquillity's `ServerUtils.SerializeResult`. The documents are committed under [fixtures/asset](../fixtures/asset/README.md). The Linux live workflow runs `asset verify`. `ferroan-services` tests encode to those bytes and serve them from a copy of the AssetStore SQLite fixture. The live workflow also runs `cargo xtask live compare assets`, comparing texture and inventory HTTP with the C# and Rust asset services, `cargo xtask live compare accounts`, comparing user-account HTTP the same way, `cargo xtask live run assets grid --rust assets`, and `cargo xtask live run login grid --rust accounts` together with `--rust assets,accounts`. The maintainer's Firestorm G0 check is recorded separately in [TASKS.md](../TASKS.md); no Aurora viewer check is claimed.

## Known differences

Three tests assert that the committed C# fixtures still show [TB1, TB10 and TB11](TRANQUILLITY_BUGS.md). A pin move that regenerates a fixture without the bug fails the test. The failure names the TB row, the fixture and the Rust code that still follows the old behavior.

- TB1: [fixtures/asset/unnamed-flags.json](../fixtures/asset/unnamed-flags.json) records `XmlSerializer` rejecting flags 8. `csharp_unnamed_asset_flags_still_reject` names `crates/ferroan-services/src/asset/xml.rs` `flags_text`. The file is not a `manifest.json` case, and its addition does not change that manifest's revision line.
- TB10: `fixtures/http-status/grid-register-302.json`, `grid-register-500.json`, `grid-deregister-302.json` and `grid-deregister-500.json` still have `outcome.accepted` true. `csharp_make_post_request_still_accepts_grid_failure_status` names `crates/ferroan-region/src/grid.rs` `grid_form` and `FormStatusPolicy::Unchecked`.
- TB11: [fixtures/pin3/lifecycle.json](../fixtures/pin3/lifecycle.json) key `pre_root_endpoint_reconnect.after` still drops the watcher, Presence session and online state while the circuit and scene presence stay. `csharp_pre_root_other_endpoint_still_drops_watcher_and_online` names `crates/ferroan-client/src/circuit.rs` `handle`.

## Session captures

`cargo xtask pin3 generate` and `verify` run the actual pinned C# login watcher, close callbacks, GridUser connector and trusted-host parser. [Pin3 fixtures](../fixtures/pin3/README.md) distinguish captured behavior from injected DNS answers, service responses and clocks. Rust tests compare lifecycle states, startup request bytes, host parsing and refresh decisions to those observations. Separate tests enforce bounded DNS work, cancellation throttling and concurrent snapshot access. This oracle builds its own scratch source copy; `FERROAN_REF` remains read-only. [The source review](PIN3_REVIEW.md) explains which changed C# paths remain outside the implemented Rust scope.

`ferroan-capture check` decodes every UDP record in a capture and requires the same bytes back. The steps, the Tranquillity patch and the redaction rules are in [CAPTURES.md](CAPTURES.md). Ordinary CI runs synthetic packet tests, local HTTP proxy tests, and `the_harness_session_replays` on [fixtures/captures/session.cap](../fixtures/captures/session.cap). The Linux live workflow records fresh Tranquillity sessions and checks them before redaction and replay. Raw captures are not committed; committed files are the reviewed, scrubbed encoding. Passing the existing fixtures does not establish support for packet variants absent from them.

The region circuit's ACK, handshake and logout send paths are checked against [C# circuit fixtures](../fixtures/circuit/README.md) at the current pin. The initial ACK/handshake test verifies sequences zero and one directly. Established ACK and logout tests drive the captured inbound packets with no sequence-counter or ACK-queue seeding, then compare every wire byte except the four-byte outbound sequence: C# emits intervening scene packets that remain planned in Rust. The tests separately assert the resulting Rust send sequence. Injected I/O errors exercise recovery independently of platform-specific ICMP timing. Shared reliable-window tests distinguish the bot's count-limited policy from the server's until-ACK policy.

Paused-clock circuit tests cover the 1000 ms initial RTO, measured ping filtering, timeout changes applied to already-pending packets, retries without a count cutoff, pause/resume, inactivity and traffic lasting beyond five minutes. `cargo test -p ferroan-client --test login_loss` runs the actual server and bot through a bounded UDP relay, dropping handshake or arrival for one second. It requires retransmission of the original bytes and sequence, fresh readiness and `LogoutReply`; both cases failed with the old retry cutoff. Region tests send a second HTTP `CreateAgent` for the same agent and prove the old circuit is canceled without clearing its replacement's CAPS.

The [CAPS fixtures](../fixtures/caps/README.md) come from a C# grid probe: seed string and URI names, malformed and non-POST requests, five XML binary cases, and an empty poll held for 58 seconds. The binary cases distinguish invalid/unpadded base64, unsupported encoding with text or explicit-empty content, and C#'s accepted self-closing exception. HTTP tests compare the captured seed bytes and headers; tests with a paused clock check the poll deadline, unchanged empty-poll ids, delivery and queue closure. The live job runs `cargo xtask live run caps grid` against C# and repeats it with `--region ferroan`, keeping the bot circuit active throughout each hold. Firestorm checks remain the maintainer's gate.

## Live harness

The [arrival fixtures](../fixtures/arrival/README.md) capture C#'s reliable `AgentMovementComplete` after `CompleteAgentMovement`. Tests compare the complete datagram after identity and timestamp folding, and exercise wrong circuits, duplicate movement requests, retransmission and stale bot readiness. `run_until_ready` requires a fresh arrival matching the current circuit's agent/session and handshake generation. The standalone and grid C# scenarios and Rust-region login run with this same requirement.

The original arrival send-path test supplies the captured C# position and channel; it proves encoding and circuit behavior. P2.0d3 additionally constructs the actual region arrival from `RegionConfig` and the captured CreateAgent, allowing only Z (terrain is pending) and the truthful Ferroan channel to differ after identity/time folding. [Region parity fixtures](../fixtures/region-parity/README.md) capture six C# admission/version responses and the Presence/GridUser service exchanges. Rust-region live login checks that logout makes GridUser offline and Presence empty, then completes a second login/logout and repeats those checks. Configuration tests cover all region INIs, map precedence, authentication rejection and control-plane trust, including mapped IPv6 and bounded DNS.

P2.0d4 adds [root-report fixtures](../fixtures/root-arrival/README.md) and a live observer before logout. The observer checks current identities, online state, region and exact stored position against the received arrival in C# and Rust. Tests hold backend replies while requiring UDP ACKs, retransmission and pings to continue; arrival waits for the reports, and duplicate movement does not report again. Dispatcher tests cover nonfatal reporting errors, old/new generations, shutdown, and logout during pending writes. Failed online observations still attempt logout. Terrain-adjusted Z remains an explicit cross-server difference until P2.0e.

P0.10a is `ferroan-bot`: XML-RPC login, the circuit and the seed capability, checked without a region (`cargo test -p ferroan-bot`). `cargo xtask live build`, `up` and `down` build the pinned Tranquillity region into `.scratch/live/` and start it on `127.0.0.1` (P0.10b). `cargo xtask live run <scenario>|all` logs in, walks, chats, builds, teleports, uploads a texture and fetches inventory, then stops the region (P0.10c). The capture patch and `bind_ip_address` are in [tools/live/README.md](../tools/live/README.md). `cargo xtask live run` records UDP and HTTP, then runs `check`, `redact` and `replay`. `cargo xtask live record-twice` records standalone `login` twice on one region and compares the captures per message type (P0.10n5). `cargo xtask live compare assets` compares texture and inventory HTTP between the C# asset service and `ferroan-robust`. `cargo xtask live assets` is that same comparison. `cargo xtask live compare accounts` compares user-account HTTP, and `cargo xtask live accounts` is that same comparison. `cargo xtask live run assets grid --rust assets` runs the grid texture and inventory scenarios with assets served by `ferroan-robust`. `cargo xtask live run login grid --rust accounts` and `--rust assets,accounts` log in through the remote account connector. `cargo xtask live up --assets ferroan` leaves a region and the Rust asset service up for a viewer; `cargo xtask live run assets --running` checks texture and inventory on that same setup, leaving both processes running until `live down`.

[`.github/workflows/live.yml`](../.github/workflows/live.yml) runs every oracle `verify`, `cargo xtask live build`, `cargo xtask live record-twice`, standalone and grid `live run all`, strict Rust-region login, C# and Rust CAPS probes, all three Ferrobot variants, `live compare assets`, `live compare accounts`, `live run assets grid --rust assets`, and `live run login grid --rust accounts` with `--rust assets,accounts`. It also checks the persistent setup with `live up --assets ferroan` and `live run assets --running`, followed by `live down` even on failure. It runs nightly, on `workflow_dispatch`, and on every pull request. `main` requires it and the three ordinary CI checks before a merge. The ordinary CI job does not start Tranquillity.

Ferrobot is the pinned external second test client ([D17](DECISIONS.md), P0.10h). Run `cargo xtask live run ferrobot`, `cargo xtask live run ferrobot grid`, and `cargo xtask live run ferrobot grid --region ferroan`. Each requires a signed API login check and fresh captured arrival, a 120-second session with matching ping replies and completed event-queue polls, and a captured `LogoutReply` after SIGTERM. The companion bot joins only after the hold, so its traffic cannot satisfy Ferrobot's session check. C# runs also require local chat in both directions and an IM received by Ferrobot. Rust-region chat and IM remain explicitly pending. Captures undergo the same lossless check, redaction and replay as other scenarios. Ferrobot builds separately from its pinned scratch clone and is not linked into Ferroan.

The commands and pass lines are in [LIVE.md](LIVE.md#ferrobot); completed local and CI runs belong in [TASKS.md](../TASKS.md), and unrun acceptance remains pending. Automation comes first ([D15](DECISIONS.md)): Firestorm is used by hand at phase gates, following [MANUAL_CHECKS.md](MANUAL_CHECKS.md). `ring` is allowed by [D16](DECISIONS.md).

The persistent mixed-grid check, `cargo xtask live persistent-grid-check`,
executes `live up grid --region ferroan` as a separate process and parses its
printed login URL and credential file path. Two fresh bot logins require
LogoutReply and bounded GridUser/Presence cleanup between sessions. It then
executes `live down` and verifies the recorded processes and their TCP/UDP
listeners are gone. This runs in the live CI job; viewer rendering remains a
maintainer check.

## Evidence by kind of change

| Change | Evidence expected |
| --- | --- |
| Parsing or encoding (packets, LLSD, XML, archives) | Tests against bytes captured from Tranquillity, plus malformed and oversized input. A round trip through our own encoder and decoder alone is not enough |
| A grid service | Captured-traffic fixtures, then a live-harness run with the C# region using the Rust service, compared with the C# service |
| Region behavior | A `ferroan-bot` scenario or Ferrobot check in the live harness. `cargo xtask live run login grid --region ferroan` logs in through C# Robust onto the Rust region, requires arrival and `LogoutReply`, and compares its registration and simulation HTTP with C# captures. The CAPS probe and Ferrobot login/session/logout subset also run against Rust; other scenario behavior uses C# regions. Firestorm by hand at the phase gate |
| Configuration | Resolved key/value sets from the Nini oracle, including the shipped region and grid examples when the reference checkout is present |
| Database access | Tests on SQLite databases created by Tranquillity's migration code. MySQL and PostgreSQL use the same check when `FERROAN_MYSQL_URL` or `FERROAN_POSTGRES_URL` is set |
| Native code (planned physics and scripting crates) | `SAFETY` comments on every unsafe block, and fuzzing of the boundary |

## Containers and monitoring

`cargo xtask docker build` builds `ferroan-robust` and `ferroan-region` from `deploy/docker/`, and the pinned Tranquillity GridServer and RegionServer images (`FERROAN_REF` or `~/Git/reference`; otherwise a scratch clone of the pin). The region image then adds `libfontconfig1`, which `libSkiaSharp.so` needs; the pin Dockerfile is not modified. It does not push them. `cargo xtask docker smoke` builds the same images, checks that each image's default user is not root, restarts `ferroan-robust` and checks that the mounted asset database still holds the library texture, then starts the example grid on the host network: C# Robust, a C# neighbour region, `ferroan-robust` for assets, and `ferroan-region`. `ferroan-bot` logs in and Ferrobot runs its login, connected hold, and logout. Local chat, instant messages, and Firestorm stay pending. Smoke writes generated passwords to `deploy/compose/.env`, which git ignores. `deploy/compose/compose.yaml` is the same four-service layout. The Linux check builds the images without pushing. The live job runs `docker smoke` after `live down`. Publishing to a registry is not part of this check.

The Ferroan grid login checks `/simstatus`, `/healthz` and `ferroan_region_root_agents` on the region metrics listener (one root agent while the session is up, none after logout). `docker smoke` scrapes `/metrics` from `ferroan-region` and `ferroan-robust` before the grid stops. `deploy/monitoring/` is the Prometheus scrape, alert rules and Grafana dashboard wired into the compose example. Prometheus and Grafana are not started by smoke.

`docker smoke` does start Loki (`grafana/loki:3.2.1`) and `otel/opentelemetry-collector-contrib:0.128.0` on the host network, and it sets the Ferroan region's `[Monitoring] LogFormat = json` and `OtlpEndpoint = http://127.0.0.1:4318` for that run only. It requires `login accepted` with `first_name=Test` and `last_name=User` in `docker logs` and in `docker compose -p ferroan-grid logs ferroan-region`, and the same login line plus `trace span=login` in Loki. The other files in [deploy/monitoring/collector/](../deploy/monitoring/collector/README.md) are checked with `otelcol validate` and are not started. A Ferroan live run also searches the region log for that run's generated passwords, session ids and seed capability path. `cargo xtask console-client` builds `OpenSim.ConsoleClient` from the pin (`FERROAN_REF` or `~/Git/reference`) and points it at a local remote console. It skips where `dotnet` or the pin is absent, and it is not part of `cargo xtask check`.

### Trying logs on one machine

Leave the keys unset and a server writes plain lines to stdout and stderr. `INFO` and below go to stdout. `WARN` and `ERROR` go to stderr. The plain line is `yyyy-MM-dd HH:mm:ss,fff`, then the level padded to five characters, then the message, matching Tranquillity's console sink.

`LOGDIR=/tmp/ferroan-logs` also writes a UTC daily file. The region file is `ferroan-regionYYYYMMDD.log` and Robust's is `ferroan-robustYYYYMMDD.log`. Tranquillity's own files are named `OpenSim.Server.*.log`. Ferroan writes the file only when `LOGDIR` is set. The images set it to `/app/logdata`.

`[Monitoring] LogFormat = json` writes one JSON object per line, with `time` (UTC, millisecond precision), `level`, `target`, `server`, `message`, and on a region `region_name` and `region_id`. `agent_id` is present only when the event has one. `[Startup] LogFormat` is used when Monitoring does not set it. `plain` and `text` are the other accepted values.

`[Monitoring] OtlpEndpoint = http://127.0.0.1:4318` exports logs, metrics and traces as OTLP/HTTP JSON to `{endpoint}/v1/logs`, `/v1/traces` and `/v1/metrics`. Nothing is exported when the key is absent. An address with credentials, or an `https` URL, is refused: this build speaks HTTP JSON only. A span named `login` is also written as a log line `trace span=login trace_id=...`, because the example collector stores logs in Loki and keeps the span itself on the collector's debug exporter.

`[Logging]` keys are category prefixes. `ferroan_region` matches `ferroan_region::simulation`. `Default` is the floor, and the floor is `INFO` when that key is absent. The remote console command `set log level` changes the floor. It does not remove a category line. `get log level` prints the floor.

`[Network] ConsoleUser` turns the remote console on. `ConsolePass` is the password, compared exactly, including empty. With no user, `/StartSession/`, `/SessionCommand/`, `/ReadResponses/{id}/` and `/CloseSession/` are not registered. `ConsoleAllowedOrigin`, when set, is sent as `Access-Control-Allow-Origin`.

`deploy/compose/compose.yaml` includes `loki` and `otel-collector` next to the grid. Grafana's provisioning adds a Loki datasource at `http://127.0.0.1:3100` and leaves Prometheus as the default. To point the collector at another system, change its config file as [the collector README](../deploy/monitoring/collector/README.md) describes. The contrib collector has no GELF exporter. The Graylog file uses Graylog's OTLP/gRPC input.

No log line, JSON field, console reply or OTLP record is supposed to carry a password, session id, secure session id, circuit code or capability URL. Those values are replaced with `[redacted]` or `[redacted-capability]` after they have been registered. The agent id may be logged. Do not put account passwords in `compose.yaml`. Keep them in the mounted INI files.

### Trying a limit hit

The ids and the C# comparison are in [LIMITS.md](LIMITS.md). Three of them can be tripped against a Ferroan region that was started with `[Monitoring] Metrics = true` and `Listen = "127.0.0.1:9109"`. The harness region trusts `127.0.0.1` and serves HTTP on `127.0.0.2`, so the POST must be bound to `127.0.0.1` or the control-plane check answers 403 before the body limit.

1. POST `http://127.0.0.2:<http-port>/agent/00000000-0000-0000-0000-000000000001/` with `Content-Length` 1048577 (1 MiB + 1) from a socket bound to `127.0.0.1`. The status is 413.
2. Send one UDP datagram of 4097 bytes to `127.0.0.1:<udp-port>`.
3. After a login, POST `FetchInventoryDescendents2` with 65 folder maps. The status is 400.

`cargo xtask live run limits grid --region ferroan` does all three and checks the readouts.

The warning is on stderr and in the `LOGDIR` file. In plain mode the line contains `limit http.post-body`, `limit udp.datagram` or `limit inventory.fetch-folders`. JSON mode adds `"target":"ferroan::limit"`. The same minute's later hits are one line with `suppressed=`. The hex excerpt of a malformed packet is a debug line, so `set log level debug` is required before it is visible.

`GET http://127.0.0.1:9109/metrics` includes `ferroan_limit_hits_total{limit="http.post-body",action="refuse"}`. Robust's listener is `127.0.0.1:9110` when its metrics switch is on. The snapshot is `diag dump` on the remote console (it prints `diag dump` and the file path), `GET /diag` on that metrics listener, and `cargo xtask live diag`. The file is `diag-<server>-<unix-millis>.txt` under `LOGDIR`, or under `.scratch/` when `LOGDIR` is unset. The live scenario writes `.scratch/live/diag/limits.txt`. A panic writes `panic-<server>-<unix-millis>.txt` beside it.

For harness changes in P0.10g, run `cargo xtask live run all grid` three consecutive times locally and once in CI. Interrupt a separate grid run after both regions have started, verify another start refuses its surviving processes, then use `cargo xtask live down` and verify those processes exit. Unit tests cover reply holds beyond the former proxy and bot timeouts, stale return-hop evidence, destination changes after opening a circuit, process ownership mismatches, and passwords spanning a log-tail cut.

## Honest status

`next_scenario_login_waits_for_griduser_and_presence_to_settle` runs the grid
scenario scheduler against controlled HTTP service replies. It first leaves
only GridUser online, then only Presence populated, then holds the final empty
Presence reply. The next scenario's login is attempted only after that reply
is released. The live grid run covers the same boundary against pinned C#.

P0.10k checks standalone followed by grid mode and verifies three separate SQLite files, interruption during standalone startup followed by `live down`, forced proxy failure, and 200 repeated `grid_state` test runs with the default parallel test runner. CAPS fixture comparisons serve the production `CapsRegistry::router()`. The event-poll hold and proxy default timeout are checked against the C# manifest's measured hold; transport connection headers and event URL shape come from that capture too. Circuit, arrival and CAPS extraction require the pinned C# channel/version on the matching login circuit. P0.10l keeps that circuit only: a later `UseCircuitCode` cannot supply logout or handshake evidence, and the CAPS seed path must be the create-agent `caps_path` of that circuit. Login and region-parity fixture generation refuse a Ferroan channel even when the probe's own `source` label says `csharp`. The proxy capture stores the forwarded request (`Connection: close`), which is what C# copied onto `response_connection`. Harness probes send `X-Ferroan-Probe` and are not selected as fixtures. `live down` runs grid-state and standalone cleanup and reports every error. Each region links an allow-list of published static files, not `maptiles/`, `MeshCache/`, console history or the daily log. The full `xtask` test binary then passed 200 consecutive runs with the default parallel runner (100 tests and 1 ignored on each run, 254s). Two earlier failures in that series, an empty `/proc` cmdline just after exec and an ephemeral UDP port released before rebind, are recorded under Found while working and fixed before this run.

A compile check proves the code builds. It does not prove a viewer can log in, a script behaves as it did in C#, or a grid stays up. Record those as pending until someone has seen them work, with the revision, viewer or grid, and result. A passing live-harness run is evidence of protocol behavior seen by its test client; it is not evidence that Firestorm renders the result, which the phase-gate check covers.
