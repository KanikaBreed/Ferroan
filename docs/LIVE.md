# Live harness

The live harness (P0.10 in [ACTION_PLAN.md](ACTION_PLAN.md)) runs a real Tranquillity region on this machine and drives it with `ferroan-bot`. It is Linux-only. Run state, the scratch Tranquillity tree and the generated password stay under `.scratch/live/`. The password is not printed. The reference checkout is only read. Set `FERROAN_REF` when that checkout is not `~/Git/reference`.

Pin8 is [KanikaBreed/OpenSim-Tranquillity](https://github.com/KanikaBreed/OpenSim-Tranquillity) `dev-build` at `c401bceecd1684832ae8ba86a31b5ee01a4ded7b` ([D21](DECISIONS.md)). Pin7 was OpenSim-NGC `develop` `1052cbae82`. Pin6 was KanikaBreed/OpenSim-Tranquillity `dev-build` `1d7abb8337`. Pin5 was that fork's `dev-build` `c70fee205f`. Pin4 was OpenSim-NGC `develop` `f67db54399`. Pin3 (`40432e0`, the maintainer's `dev-build`) merged in [#56](https://github.com/KanikaBreed/Ferroan/pull/56). The clean shared reference checkout is at Pin8. CI reads both the clone URL and revision from [REFERENCES.md](REFERENCES.md). Builds use separate writable scratch copies; `FERROAN_REF` selects clean alternate reference checkouts when needed.

The fixed C# sends `LogoutReply` immediately. The strict login checks still require that packet and a fresh arrival on the current circuit. The Rust region also follows pending ViaLogin connections, clears stranded GridUser users at startup, and attempts independent close notifications. Controlled C# observations for those cases and trusted-host parsing are regenerated with `cargo xtask pin3 generate` and checked with `cargo xtask pin3 verify`; see [the fixture provenance](../fixtures/pin3/README.md).

`cargo xtask live build` clones the pinned revision, applies the capture patch in [tools/live/README.md](../tools/live/README.md), and publishes the region server and a self-contained GridServer. `cargo xtask live up` starts the region on `127.0.0.1` and prints the login URL. Add `--assets ferroan` to start `ferroan-robust` on a seeded asset store and point the region at it. `cargo xtask live down` stops the region and its asset process. `cargo xtask live run <scenario>|all` builds if needed, runs the scenarios, records UDP and HTTP, and runs `check`, then `redact`, then `replay`. A failing check fails the run. `cargo xtask live record-twice` runs standalone `login` twice on that one region and compares per-type counts, the first appearance of the handshake and logout messages, and the stable fields. A reliable message counts once per direction and sequence number, so a resend does not add to that count, and the report states how many resends each type had. `cargo xtask live run <scenario>|all|neighbour grid` starts GridServer and two neighbouring regions instead of one standalone region. `cargo xtask live compare assets` (and `cargo xtask live assets`, the same command) runs texture and inventory against the C# asset service and against `ferroan-robust`, then compares the normalized HTTP. `cargo xtask live compare accounts` (and `cargo xtask live accounts`, the same command) posts the same user-account forms to the C# connector and to `ferroan-robust` and compares the replies after folding `Created` and `NameChanged`. Plain `live up` uses the in-process asset service. Neither `live up` mode records a capture.

`live build` also builds the pinned Ferrobot with `cargo build --locked`. A clean checkout at `FERROAN_REF/Ferrobot` supplies a scratch clone when its revision matches [REFERENCES.md](REFERENCES.md); otherwise the harness clones the public repository at that pin. Source and build output remain in `.scratch/ferrobot-src` and `.scratch/ferrobot-target`. An existing scratch checkout with changes or a different revision is refused and preserved for inspection. No reference checkout is built in. Ferrobot is a separate executable, with no vendored code or dependency from a Ferroan crate.

## Scenarios

`all` runs these, in order. Each line is the report the command prints when the check passes.

Every harness login waits, for at most ten seconds, until GridUser shows the
account offline and Presence has no session for it. A missing griduser row
(`Failure` from `getgriduserinfo`) counts as offline, so the first login is
not held. That includes texture and
then inventory on a standalone region, and the wait between grid scenarios.
Chat also waits for the second account. The probe reads Robust in grid mode
and, in standalone, the region's own Presence and GridUser handlers
(`ServiceProbe` in the capture patch; see [tools/live/README.md](../tools/live/README.md)).
When the bound runs out, the login is not attempted. LogoutReply remains
required wherever strict logout is enabled. At Pin3 the reply can arrive before
asynchronous service cleanup, so it alone cannot authorize the next login.

| Name | Pass line |
| --- | --- |
| login | `login: pass` |
| walk | `walk: pass` with the start and end position |
| chat | `chat: pass`. A second account, `Chat Bot`, hears `ferroan harness chat` |
| build | `build: pass`. A prim is rezzed, changed and deleted |
| teleport | `teleport: pass` with the landing position |
| texture | `texture: pass` with the uploaded size and the library texture size. Both are JPEG 2000 |
| inventory | `inventory: pass`. The skeleton and a folder fetch include the uploaded item |

`assets` is texture and inventory, used by `cargo xtask live compare assets` and by `cargo xtask live assets`, which is the same comparison. `accounts` is the user-account form comparison, used by `cargo xtask live compare accounts` and by `cargo xtask live accounts`. It does not start regions or bots. `cargo xtask live run login grid --rust accounts` and `--rust assets,accounts` serve those tables from `ferroan-robust` on a separate SQLite file. C# Robust login uses the remote account connector. `auth` is the authentication form comparison, used by `cargo xtask live compare auth` and by `cargo xtask live auth`. It does not start regions or bots. `cargo xtask live run login grid --rust auth` serves authentication from `ferroan-robust` on a separate AuthStore file. C# Robust login stays on `PasswordAuthenticationService` and opens that same file. The harness posts `setpassword` itself, because `AuthenticationServicesConnector.SetPassword` returns false.

## Ferrobot

Run the dedicated second-client scenario separately from `all`:

```sh
cargo xtask live run ferrobot
cargo xtask live run ferrobot grid
cargo xtask live run ferrobot grid --region ferroan
```

The harness starts Ferrobot as its test account. The generated password and `FERRO_SHARED_SECRET` stay in a private `ferrobot/environment.json` under the run's `.scratch/` directory. `FERRO_HTTP_BIND=127.0.0.1` and `FERRO_HTTP_PORT=0` give the API a free loopback port. Relog and IM commands are disabled. Requests to `/api/Run` follow the pinned client's API: form fields `commandName`, `args` joined with `~#~`, `unixtime`, and a SHA-1 signature over command, arguments, timestamp and shared secret. API responses and logs are bounded; diagnostics do not print generated credentials. The controller records Ferrobot's process identity so `live down` can recover an interrupted run.

Login requires the signed `SimName` response and captured circuit, handshake/reply, movement request and fresh reliable `AgentMovementComplete` for the test account. The client then stays connected for 120 seconds before the companion bot logs in. The check requires continuing matching ping replies and repeated event polls with captured valid responses: held empty 502 replies or LLSD event batches. Merely seeing poll requests is insufficient. C# runs then exchange synthetic local chat in both directions with `ferroan-bot` and deliver an IM to Ferrobot. All three variants require a matching captured `LogoutRequest` and subsequent `LogoutReply` after SIGTERM; server log lines cannot satisfy this check.

| Check | C# standalone and grid | Ferroan region |
| --- | --- | --- |
| Arrival | `ferrobot login: pass` | Same |
| Connected session | `ferrobot connected: pass (120 s; pings and event-queue polls)` | Same |
| Local chat | `ferrobot chat receive: pass`, `ferrobot chat send: pass` | `ferrobot local chat: pending (P2.0g)` |
| IM | `ferrobot IM: pass` | `ferrobot IM: pending (region IM support)` |
| SIGTERM logout | `ferrobot logout: pass (SIGTERM; LogoutReply)` | Same |

These are required report lines, not a claim that an unrun validation passed; [TASKS.md](../TASKS.md) records completed local and CI evidence. Firestorm validation remains separate.

Ferrobot's region HTTP passes through capture proxies; C# UDP uses the existing capture patch. The Rust-region variant uses a bounded loopback UDP relay for its single test circuit. Captures must pass `check`, `redact` and `replay` without filtering unsupported packets. The relay records original datagrams before forwarding, and the private raw capture stays in scratch. API control requests are not part of region-traffic fixtures. Ferrobot bugs must be recorded and reported upstream with the maintainer's agreement, never patched or silently worked around here.

## Keeping the Ferroan region up

```sh
cargo xtask live build
cargo xtask live up grid --region ferroan
```

This leaves C# Robust, the C# neighbour and the Ferroan home region running for
the maintainer's [informal Firestorm checks](MANUAL_CHECKS.md#firestorm-on-the-ferroan-region-before-g2).
Use the printed Robust login URL, account name and password file path. The
report also identifies each process's ports and the Ferroan commit. The
password stays in scratch and is never printed. These direct listeners do not
record captures. All listeners default to `127.0.0.1`.

To use a trusted test network explicitly, add `--bind <IPv4>` with a concrete
address assigned to this machine. The command binds and advertises that address
and warns that the test grid, including Robust's private services, is exposed
beyond loopback. Wildcard, broadcast and multicast addresses are refused.
Automated checks use loopback; external-network access is not part of them.

Run `cargo xtask live down` from the same worktree after logging out, or to
recover an interrupted launch. Stop this setup before starting other live
commands. `cargo xtask live persistent-grid-check` automates startup, two bot
logins through the printed URL, strict LogoutReply checks and bounded service
cleanup between sessions, then shutdown and process/listener cleanup checks.
The CI live job runs this check. Firestorm observations remain pending until
the maintainer records them; this command does not pass G2.

## Keeping the Ferroan asset service up

```sh
cargo xtask live build
cargo xtask live up --assets ferroan
cargo xtask live run assets --running
cargo xtask live down
```

After the build, `live up --assets ferroan` seeds the library by starting `ferroan-robust` with `DefaultAssetLoader` pointed at the pin's `AssetSets.xml`, then copies that SQLite store for the serving `ferroan-robust` (loader off) and starts the asset service and region on loopback. The region's `Include-Asset` override selects the Rust service after `Standalone.ini` is loaded. The command prints the login URL and both service ports and leaves the processes running. The account is `Test User`; its generated password stays in `.scratch/live/run/password`.

`live run assets --running` uses that existing region and the saved credentials to check a texture upload, a library texture fetch, and inventory through `ferroan-bot`. It leaves both services running. Use this before the [Firestorm G0 checks](MANUAL_CHECKS.md); run `live down` after the viewer logs out. The normal `live run` commands start their own regions and require the persistent setup to be stopped first.

`live compare assets` is the C#/Rust comparison, and `live assets` is the same command: each starts the asset service in turn, runs the same texture and inventory scenarios, and requires their normalized HTTP to match. `live compare accounts` does the same for user-account forms, folding `Created` and `NameChanged`, and `live accounts` is that command. `live compare auth` does the same for authentication forms, folding the issued token, and `live auth` is that command. The C# side of that compare names a scratch asset SQLite file so `AuthorizationService` can start; Ferroan opens no asset database for that section (D34). The C# asset-service configuration explicitly sets `[Network] bind_ip_address = "127.0.0.1"`; an unset address binds every interface and fails the harness's loopback check. `--db sqlite` is the default. `--db mysql` and `--db pgsql` stop before any service starts and say they need P1.A1. The authentication compare stays on SQLite.

## Grid mode

`cargo xtask live run all grid` starts Tranquillity GridServer (Robust) and two regions, `Ferroan Test` at 1000,1000 and `Ferroan Neighbour` at 1001,1000. The regions use the shipped `Grid.ini` and a copy of `GridCommon.ini.example`. That copy stays unchanged unless `--rust` names a service. `--rust assets` starts `ferroan-robust` on `127.0.0.1` with the seeded SQLite store, omits C# Robust's asset connector, and writes each region its own `GridCommon.ini` whose `AssetServerURI` is the Rust service. `--rust accounts` uses a separate copy of the UserAccount fixture on that process, omits `UserAccountServiceConnector`, points `[LoginService] UserAccountService` at `OpenSim.Services.Connectors.dll:UserAccountServicesConnector`, and sets `UserAccountServerURI` to the same URI. `--rust auth` uses a separate copy of the AuthStore fixture, omits `AuthenticationServiceConnector`, and sets `AuthenticationServerURI` to the Rust service. `[LoginService] AuthenticationService` stays `PasswordAuthenticationService` and its connection string is that same `auth.db`, because `LLLoginService` loads the plugin with `(config, userAccountService)` and `AuthenticationServicesConnector` has no such constructor. When accounts stay in process, `[UserAccountService] AuthenticationService` is the connector. Regions already read `AuthenticationServerURI` through `RemoteAuthenticationServicesConnector`. The harness posts `setpassword` to the Rust service, because the connector's `SetPassword` returns false. `assets,accounts` is one process and both keys. Other services stay on C# Robust. `cargo xtask live run assets grid --rust assets`, `cargo xtask live run login grid --rust accounts` and `cargo xtask live run login grid --rust auth` are the grid checks. The published example files are not edited. Each region has its own working directory, so the relative SQLite files in `SQLiteStandalone.ini` stay separate. Robust listens on `127.0.0.4`. Each region listens for HTTP on `127.0.0.2` or `127.0.0.3` and advertises `127.0.0.1` at the same port, where a `ferroan-capture` proxy is bound. UDP stays on `127.0.0.1`. The bot logs in through Robust's own public port. The same scenarios as standalone run, then a teleport to the neighbour and back prints `teleport neighbour: pass (1001,1000 and back)`. The bot polls the source region's `EventQueueGet`. The first hop opens the destination with `UseCircuitCode` when `EnableSimulator` names that handle, and sends `CompleteAgentMovement` only after `TeleportFinish`. The source circuit stays open. The return hop sends `CompleteAgentMovement` on that circuit instead of a second `UseCircuitCode`. The hop waits for the destination's `AgentMovementComplete` before the return trip. `neighbour` runs only that teleport. The run fails unless the Robust private proxy and at least one region proxy recorded an HTTP request. Standalone `live up` and `live run` are unchanged.

Standalone, grid and asset runs save their controller and child process identities under `.scratch/live/grid-state`. An interruption can leave services alive because they have separate process groups. After interrupting the command, run `cargo xtask live down` to stop its recorded processes, including a service interrupted before startup completed. A new harness start refuses surviving processes instead of replacing their files. Cleanup checks process start time, executable and command fingerprint before signalling a saved process; it refuses to race a still-running controller. Before each spawn, the controller saves a unique launch marker and expected executable, working directory and command fingerprint, then records the child before waiting for readiness. This lets cleanup recover a child even if interruption happened before its PID was recorded, without adopting an unrelated process running the same command.

Recovery skips unrelated unreadable process environments and those exceeding 64 KiB. A possible owned child with inaccessible identity, or the overall scan limits (32,768 entries, 16 MiB, five seconds), preserves state and reports an error. If that persists, interrupt the controller and inspect `.scratch/live/grid-state`. Verify each recorded or pending child's start time, executable, working directory and process group before stopping it. Only after every owned child is confirmed gone, archive the state file elsewhere under `.scratch/` and run `live down` again. Do not remove the lock file while a controller can hold it, or stop unrelated processes to reduce the scan.

The grid's runtime links exclude standalone SQLite databases and their journal/WAL sidecars, so standalone followed by grid leaves three separate region stores. The harness builds Rust binaries with Cargo's JSON artifact output and uses the reported executable, including `CARGO_TARGET_DIR` or Cargo configuration overrides. Secret files are restricted to mode 0600 on every write, including existing files.

Every grid and asset proxy permits a complete upstream reply to take 70 seconds, longer than Tranquillity's 58-second event-queue hold. The bot permits a poll to take 65 seconds. The proxy explicitly closes each connection after one exchange; captured headers are the originals received from each endpoint. Log diagnostics redact every known generated password before selecting the last 4000 bytes.

Interim `1xx` responses do not end an exchange or reset its deadline. Any exchange failure fails the proxy and harness run; shutdown drains accepted exchanges before reporting success. The proxy CLI also exits unsuccessfully on exchange failure.

The return teleport requires a fresh `RegionHandshake` and `AgentMovementComplete`; a region UUID left from the outbound circuit is insufficient. A hint that changes the destination after its circuit is open is rejected. The one-second grace after arrival remains: C#'s `IncomingUpdateChildAgent` polls the root flag every 250 ms, and an immediate return could make the avatar a child again before that operation notices arrival. This grace is separate from the corrected proxy timeout.

## Login onto the Ferroan region

Readiness on both C# and Rust requires a fresh reliable `AgentMovementComplete` for the current circuit, in addition to its handshake and seed. `cargo xtask live arrival-fixtures <udp-capture>` extracts the request/reply and manifest in [fixtures/arrival](../fixtures/arrival/README.md) from a C# login capture. It folds test identities and the wall-clock timestamp; the captured position, look direction, region handle and channel/version remain intact. The Rust region preserves the `CreateAgent` start position until terrain is implemented and advertises its own server version.

`cargo xtask live trailing-fixtures <udp-capture>` extracts the C# `ImprovedInstantMessage` evidence in [fixtures/trailing](../fixtures/trailing/README.md). It requires the pinned C# channel/version and harness IM marker, checks the original capture before redaction, and retains the original packet length and viewer-defined `EstateBlock` suffix. The fixture was selected from the preserved Q10 candidate's standalone capture.

For the first Ferroan region login, run `cargo xtask live run login grid --region ferroan`. Robust and the neighbour remain C#; only the home region is Ferroan. The bot requires `LogoutReply`, with no C# log-line fallback. The harness compares the region's registration and Robust's `QueryAccess`/`CreateAgent` requests and replies with scrubbed C# fixtures. The seed advertises `EventQueueGet`, with Tranquillity's 58-second empty-poll hold and 502 response. When the client asks, the same seed also returns the configured inventory and asset fetch capabilities. `cargo xtask live run assets grid --region ferroan` runs the texture and inventory scenarios on the Ferroan home region and compares those fetches with the C# neighbour. The login scenario also requires one avatar `ObjectUpdate` and one unbaked `AvatarAppearance` for the logged-in agent.

The region uses C#'s configurable retransmission and inactivity timers; an active circuit has no fixed lifetime. `cargo test -p ferroan-client --test login_loss` separately drops the handshake or arrival for one second through a UDP relay and requires the bot to finish login and logout. Same-agent replacement and generation-scoped CAPS cleanup are covered by region HTTP tests. The Rust login command now polls Robust's GridUser/Presence services for offline state, completes a second login/logout, then checks both services again. Six HTTP probes compare admission refusals and version negotiation with [C# captures](../fixtures/region-parity/README.md). `cargo xtask live run login grid` records those probes from C#; `cargo xtask live region-parity-fixtures .scratch/live/grid` regenerates the fixtures. See the [region guide](../crates/ferroan-region/README.md) for settings and D19/D20 for bounded input differences.

Both dedicated grid login commands also inspect Robust while the bot is online. Presence must name its current region; GridUser must be online with the same last region and exact `f32` position as its received arrival. The observer keeps the circuit active and attempts logout on either success or failure. `cargo xtask live root-arrival-fixtures .scratch/live/grid` captures the two C# report forms and the online observation after a C# login run. [Root-arrival evidence](../fixtures/root-arrival/README.md) describes identity folding and the still-pending terrain-Z difference; Rust is compared with its actual arrival, not an injected C# position.

The dedicated login baseline disables map generation for both C# and Rust using `[Map] GenerateMaptiles = false` and a zero static map UUID. This C# setting produces zero map image IDs during registration. It leaves map rendering outside this login task. The normal `all grid` scenarios retain their existing map configuration. The process's supported configuration and current bounds are in [ferroan-region](../crates/ferroan-region/README.md).

`cargo xtask live login-fixtures <capture-dir>` regenerates the scrubbed control-plane fixtures from a local C# login capture directory. Use only test accounts on the harness grid. The generator validates required exchange shapes, bounds decompression, and substitutes generated identities and test endpoints; raw captures stay in `.scratch/`. The fixture README records the source revision and substitutions.

`cargo xtask live circuit-fixtures <udp-capture>` extracts C# circuit evidence into `fixtures/circuit`: the immediate and established ACKs, handshake and logout reply, plus their sequence context. Run a C# login in grid mode first and use its UDP capture. The generator checks each datagram's re-encoding and applies the existing P0.10d identity scrubber; it refuses a capture missing required evidence. Tests compare actual region-side sends with these bytes, retaining reliability, zero coding and sequence allocation rules.

`cargo xtask live run caps grid` checks the C# seed and event queue, and `cargo xtask live run caps grid --region ferroan` repeats the probe with Ferroan as the home region. The probe requests `EventQueueGet` by string and URI, checks malformed and non-POST seed requests, drains startup events, and requires an empty 502 response after the hold. Binary seed cases cover invalid and unpadded base64, unsupported encoding with text or explicit empty content, and C#'s self-closing binary exception. It keeps the UDP session active throughout. Both run in live CI; `caps` is a dedicated probe rather than part of `all`.

`cargo xtask live caps-fixtures <capture-dir>` extracts the seed and event-poll HTTP from a C# probe run into `fixtures/caps`. Its manifest records response status, content type, Connection headers, event URL shape and elapsed time. Generated capability URLs and identities are substituted; response bytes otherwise stay as captured. The directory must also contain the corresponding `capture-udp.cap`. Circuit, arrival and CAPS extractors require the exact pinned Tranquillity channel/version on the matching login circuit, since the Rust region shares the test account and region identifiers. Raw captures remain under `.scratch/`.

## Adding a scenario

Add the name to `NAMES` and the `selection` match in [tools/bot/src/scenario.rs](../tools/bot/src/scenario.rs), and implement the check there. The function returns the report line, or an error whose text is the failure. `cargo xtask live run <name>` runs that one scenario. `all` runs every name in `NAMES`. The new scenario's captured chat must be synthetic. Do not put a password, a session id or a circuit code into the message text. The redacted session committed under `fixtures/captures/` is produced by `live run all`.

## What CI runs

[`.github/workflows/live.yml`](../.github/workflows/live.yml) runs the Linux harness as four jobs at the same time: scenarios (the oracles, the harness, and the seed and event-poll probes), Ferrobot, the login and service comparisons (including `live up --assets ferroan`, `live run assets --running`, and `live down` even when an earlier step in that job fails), and the container images with `cargo xtask docker smoke`. A final job named `live (ubuntu-latest)` runs `cargo xtask ci gate` and fails unless all four succeeded. That name is the required check on `main`. `cargo xtask ci scope` decides whether the pull request is documentation-only before those jobs start. The Ferrobot job runs `cargo xtask live build` first: standalone Ferrobot does not publish the region, and a cache miss would otherwise fail with `run cargo xtask live build first`. Each work job installs .NET SDK 10.0.114 with `actions/setup-dotnet` pinned by commit, runs `cargo xtask ci ensure-sqlite`, and clones the pinned Tranquillity, libopenmetaverse and Ferrobot checkouts into `FERROAN_REF`. `ensure-sqlite` skips `apt-get` when `sqlite3` is already on `PATH`. The images job runs on the maintainer's self-hosted runner (labels `self-hosted`, `linux`, `x64`, `ferroan`; the registered machine is Home1, a dedicated user on Arch Linux with Docker 29.7.2, Compose 5.8.0, rustup toolchain 1.100.0 and `sqlite3` on `PATH`) only when the repository variable `FERROAN_SELF_HOSTED` is `true` and the run is not a pull request from a fork. Otherwise it stays on `ubuntu-latest`. Unset the variable to turn that runner off. With the variable set, the images job takes one slot at a time (`ferroan-live-images`) because smoke uses host networking and fixed ports. Fork pull requests never use that runner. [TESTING.md](TESTING.md) records the setup. It restores from nuget.org only and needs no grid credentials. The Tranquillity publish tree and the Rust build are restored through `actions/cache` pinned by commit. The Tranquillity key is the pin, the capture patch and `xtask/src/live.rs`: the publish steps live in that file next to the scenario runners, and the cached tree is `.scratch/live/src`, so narrowing the key would miss the warm publish. The jobs run nightly, on `workflow_dispatch`, and on every pull request. A pull request that changes only Markdown, other than `README.md`, `docs/REFERENCES.md` and `docs/LIMITS.md`, skips the four jobs. The required check then runs `cargo xtask ci scope` and `cargo xtask ci gate` from the cached Rust build and does not build Tranquillity or start a server. Scheduled and manual runs always run everything. The ordinary CI job does not start Tranquillity.

The Ferrobot revision and role are recorded in [REFERENCES.md](REFERENCES.md) and [D17](DECISIONS.md). Historical blockers and their prerequisite fixes remain in [WORK_QUEUE.md](../WORK_QUEUE.md); current acceptance evidence belongs in P0.10h's task row.

## Manual recipe (Linux)

The commands above automate this recipe. It was run by hand on Linux on 17 November 2026 at Tranquillity `713857f10b8e91a639a967d4354a4682b080793c`: the region started without a console, created its estate owner, and accepted an XML-RPC login (`login: true`, seed capability on `127.0.0.1:9100`). The same recipe, built from Tranquillity PR #412 (on `develop` at `578db56`) with `bind_ip_address = "127.0.0.1"`, on databases a PR #411 build had created, was then used by the maintainer with Firestorm 7.2.3.80036: login, avatar, textures, building, upload, chat and movement all worked, with the C# asset service. That session is not the phase-gate check (G0). The commands below now select Pin8, KanikaBreed/OpenSim-Tranquillity `dev-build` `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`. The historical viewer observations are not a claim of manual validation at this pin. The harness sets `bind_ip_address` and starts the region once.

Start from a Ferroan worktree root; the commands create a separate scratch clone. Never build in `~/Git/reference`.

1. Clone and build the pinned revision. The NuGet restore uses nuget.org only and needs no credentials: Tranquillity dropped its GitHub Packages feed in `a4ebd66006` (29 September 2026). Its `Docs/BUILDING.md` still describes the old feed and its `GITHUB_TOKEN`; ignore that part.

   ```sh
   W="$PWD/.scratch/tranquillity-live"
   git clone --no-checkout https://github.com/KanikaBreed/OpenSim-Tranquillity.git "$W"
   git -C "$W" checkout --detach c401bceecd1684832ae8ba86a31b5ee01a4ded7b
   cd "$W"
   dotnet publish Source/OpenSim.Server.RegionServer/OpenSim.Server.RegionServer.csproj -c Release -r linux-x64
   P="$W/Source/OpenSim.Server.RegionServer/bin/Release/net10.0/linux-x64/publish"
   cp "$P/config-include/StandaloneCommon.ini.example" "$P/config-include/StandaloneCommon.ini"
   cp "$P/config-include/FlotsamCache.ini.example" "$P/config-include/FlotsamCache.ini"
   ```

   The build takes about a minute with a warm package cache.

2. Write the run configuration in `$W/run` (generate the password per run; never commit it):

   `$W/run/config/RegionServer.ini`

   ```ini
   [Const]
       BaseHostname = "127.0.0.1"
       BaseURL = http://${Const|BaseHostname}
       PublicPort = "9100"
       PrivURL = ${Const|BaseURL}
       PrivatePort = "9103"

   [Startup]
       regionload_regionsdir = "<W>/run/Regions"

   [Network]
       http_listener_port = 9100
       bind_ip_address = "127.0.0.1"

   [Estates]
       DefaultEstateName = "Ferroan Test Estate"
       DefaultEstateOwnerName = "Test User"
       DefaultEstateOwnerEMail = "test@example.invalid"
       DefaultEstateOwnerUUID = "4f0c8d2e-7a51-4b8e-9c3d-2e6f1a0b7c55"
       DefaultEstateOwnerPassword = "<generated>"

   [Architecture]
       Include-Architecture = "config-include/Standalone.ini"
   ```

   `$W/run/Regions/Regions.ini`

   ```ini
   [Ferroan Test]
   RegionUUID = 6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01
   Location = 1000,1000
   InternalAddress = 127.0.0.1
   InternalPort = 9100
   AllowAlternatePorts = False
   ExternalHostName = 127.0.0.1
   ```

3. Start it. Ready when the log prints `INITIALIZATION COMPLETE FOR Ferroan Test - LOGINS ENABLED` (about 45 seconds):

   ```sh
   cd "$P"
   ./OpenSim.Server.RegionServer --inidirectory "$W/run/config" --background > "$W/run/region.log" 2>&1 &
   ```

4. Log in: the login URL is `http://127.0.0.1:9100/`, the account is `Test User` with the generated password. The XML-RPC `login_to_simulator` call takes the password as `$1$` followed by its MD5 hex.

5. Stop: `kill` the process (SIGTERM), or remove `$P/*.db` and `$W/run` to start fresh.

## Gotchas found running the recipe

1. **`[Const]` needs `BaseURL`, `PrivURL` and `PrivatePort`,** not only `BaseHostname` and `PublicPort`. Without them startup fails with `Expand key not found: BaseURL`.
2. **`--background` cannot answer console prompts.** At this pin, `ConsoleBase` takes and logs a prompt's default when standard input is redirected. A prompt without a default fails with an error naming the question and asking for a configuration value. This replaces the redirected-console key error seen at earlier pins. The harness still supplies the estate owner's settings, including `DefaultEstateOwnerUUID`, explicitly. `RemoteConsole` can still answer prompts over HTTP.
3. **SQLite user columns.** At `713857f`, SQLite `UserAccount.migrations` stopped at version 3 and omitted `DisplayName` and `NameChanged`, so creating a user failed. At this pin, version 4 of that file adds both columns. PostgreSQL `UserAccount` is version 6 for the same columns. The harness starts the region once and does not alter the table.
4. **HTTP listen address.** `[Network] bind_ip_address` is read by `RegionHttpServerFactory` and `GridHttpServerFactory`. Empty or unset means every interface. The harness sets `bind_ip_address = "127.0.0.1"`. The region's UDP port still follows `InternalAddress`.
5. **Expected noise:** `Can't load experience service` (`Could not find a storage interface in the given module OpenSim.Data.SQLite.dll`). Tranquillity has no SQLite store for Experiences; the region runs without them.
6. Databases and the console history are written into the publish folder (`$P`), not into `$W/run`. A fresh run deletes `$P/*.db`.
