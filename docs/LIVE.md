# Live harness

The live harness (P0.10 in [ACTION_PLAN.md](ACTION_PLAN.md)) runs a real Tranquillity region on this machine and drives it with `ferroan-bot`. It is Linux-only. Run state, the scratch Tranquillity tree and the generated password stay under `.scratch/live/`. The password is not printed. The reference checkout is only read. Set `FERROAN_REF` when that checkout is not `~/Git/reference`.

`cargo xtask live build` clones the pinned revision, applies the capture patch in [tools/live/README.md](../tools/live/README.md), and publishes the region server and a self-contained GridServer. `cargo xtask live up` starts the region on `127.0.0.1` and prints the login URL. `cargo xtask live down` stops it. `cargo xtask live run <scenario>|all` builds if needed, runs the scenarios, records UDP and HTTP, and runs `check`, then `redact`, then `replay`. A failing check fails the run. `cargo xtask live assets` runs texture and inventory against the C# asset service and against `ferroan-robust`, then compares the normalized HTTP. `live up` uses the in-process asset service and does not record a capture.

## Scenarios

`all` runs these, in order. Each line is the report the command prints when the check passes.

| Name | Pass line |
| --- | --- |
| login | `login: pass` |
| walk | `walk: pass` with the start and end position |
| chat | `chat: pass`. A second account, `Chat Bot`, hears `ferroan harness chat` |
| build | `build: pass`. A prim is rezzed, changed and deleted |
| teleport | `teleport: pass` with the landing position |
| texture | `texture: pass` with the uploaded size and the library texture size. Both are JPEG 2000 |
| inventory | `inventory: pass`. The skeleton and a folder fetch include the uploaded item |

`assets` is texture and inventory, used by `cargo xtask live assets`.

## Adding a scenario

Add the name to `NAMES` and the `selection` match in [tools/bot/src/scenario.rs](../tools/bot/src/scenario.rs), and implement the check there. The function returns the report line, or an error whose text is the failure. `cargo xtask live run <name>` runs that one scenario. `all` runs every name in `NAMES`. The new scenario's captured chat must be synthetic. Do not put a password, a session id or a circuit code into the message text. The redacted session committed under `fixtures/captures/` is produced by `live run all`.

## What CI runs

[`.github/workflows/live.yml`](../.github/workflows/live.yml) is the Linux job. It installs .NET SDK 10.0.114 with `actions/setup-dotnet` pinned by commit, installs `sqlite3`, and clones the pinned Tranquillity and libopenmetaverse checkouts into `FERROAN_REF`. It restores from nuget.org only and has no secrets. It runs every `cargo xtask` oracle `verify`, then `cargo xtask live build` and `cargo xtask live run all`. The Tranquillity publish is cached by the pin, the capture patch and the harness source. The job runs nightly, on `workflow_dispatch`, and on pull requests that touch the harness. The ordinary CI job does not start Tranquillity.

## Manual recipe (Linux)

The commands above automate this recipe. It was run by hand on Linux on 17 November 2026 at Tranquillity `713857f10b8e91a639a967d4354a4682b080793c`: the region started without a console, created its estate owner, and accepted an XML-RPC login (`login: true`, seed capability on `127.0.0.1:9100`). The same recipe, built from Tranquillity PR #412 (on `develop` at `578db56`) with `bind_ip_address = "127.0.0.1"`, on databases a PR #411 build had created, was then used by the maintainer with Firestorm 7.2.3.80036: login, avatar, textures, building, upload, chat and movement all worked, with the C# asset service. That session is not the phase-gate check (G0). The pin is now `c85783e993736c172149d0bad63dc16ba910e62f`, which contains both fixes. The harness sets `bind_ip_address` and starts the region once.

All paths are scratch paths. Never build in `~/Git/reference`.

1. Clone and build the pinned revision. The NuGet restore uses nuget.org only and needs no credentials: Tranquillity dropped its GitHub Packages feed in `a4ebd66006` (29 September 2026). Its `Docs/BUILDING.md` still describes the old feed and its `GITHUB_TOKEN`; ignore that part.

   ```sh
   W=~/Git/work/ferroan/tranquillity-live
   git clone --no-checkout ~/Git/reference/OpenSim-Tranquillity "$W"
   git -C "$W" checkout --detach c85783e993736c172149d0bad63dc16ba910e62f
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
2. **`--background` cannot answer console prompts.** Any setting Tranquillity would ask for at first start must be in the configuration, or it dies with `Cannot see if a key has been pressed when ... console input has been redirected`. For the estate owner that includes `DefaultEstateOwnerUUID` (`OpenSimBase.SetUpEstateOwner` prompts for it when unset).
3. **SQLite user columns.** At `713857f`, SQLite `UserAccount.migrations` stopped at version 3 and omitted `DisplayName` and `NameChanged`, so creating a user failed. At this pin, version 4 of that file adds both columns. PostgreSQL `UserAccount` is version 6 for the same columns. The harness starts the region once and does not alter the table.
4. **HTTP listen address.** `[Network] bind_ip_address` is read by `RegionHttpServerFactory` and `GridHttpServerFactory`. Empty or unset means every interface. The harness sets `bind_ip_address = "127.0.0.1"`. The region's UDP port still follows `InternalAddress`.
5. **Expected noise:** `Can't load experience service` (`Could not find a storage interface in the given module OpenSim.Data.SQLite.dll`). Tranquillity has no SQLite store for Experiences; the region runs without them.
6. Databases and the console history are written into the publish folder (`$P`), not into `$W/run`. A fresh run deletes `$P/*.db`.
