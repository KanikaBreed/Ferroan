# Live harness

The live harness (P0.10 in [ACTION_PLAN.md](ACTION_PLAN.md)) runs a real Tranquillity region on this machine and drives it with `ferroan-bot`. Until `cargo xtask live` exists, this file is the manual recipe it automates. The recipe below was run by hand on Linux on 17 November 2026 at Tranquillity `713857f10b8e91a639a967d4354a4682b080793c`: the region started without a console, created its estate owner, and accepted an XML-RPC login (`login: true`, seed capability on `127.0.0.1:9100`).

## Manual recipe (Linux)

All paths are scratch paths. Never build in `~/Git/reference`.

1. Clone and build the pinned revision. The NuGet restore uses the OpenSim-NGC GitHub Packages feed; a machine that has built Tranquillity before has the packages cached, a clean machine or CI needs `GITHUB_ACTOR` and `GITHUB_TOKEN` (Tranquillity's `Docs/BUILDING.md`).

   ```sh
   W=~/Git/work/ferroan/tranquillity-live
   git clone --no-checkout ~/Git/reference/OpenSim-Tranquillity "$W"
   git -C "$W" checkout --detach 713857f10b8e91a639a967d4354a4682b080793c
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

3. Start it once so it creates its SQLite databases, then stop it (it stops by itself at the estate owner step on first run; see gotcha 3), fix the user table, and start it again:

   ```sh
   cd "$P"
   ./OpenSim.Server.RegionServer --inidirectory "$W/run/config" --background > "$W/run/region.log" 2>&1
   sqlite3 "$P/userprofiles.db" \
     "ALTER TABLE UserAccounts ADD COLUMN DisplayName VARCHAR(31) NULL;
      ALTER TABLE UserAccounts ADD COLUMN NameChanged INTEGER NULL DEFAULT 0;"
   ./OpenSim.Server.RegionServer --inidirectory "$W/run/config" --background > "$W/run/region.log" 2>&1 &
   ```

   Ready when the log prints `INITIALIZATION COMPLETE FOR Ferroan Test - LOGINS ENABLED` (about 45 seconds).

4. Log in: the login URL is `http://127.0.0.1:9100/`, the account is `Test User` with the generated password. The XML-RPC `login_to_simulator` call takes the password as `$1$` followed by its MD5 hex.

5. Stop: `kill` the process (SIGTERM), or remove `$P/*.db` and `$W/run` to start fresh.

## Gotchas found running the recipe

1. **`[Const]` needs `BaseURL`, `PrivURL` and `PrivatePort`,** not only `BaseHostname` and `PublicPort`. Without them startup fails with `Expand key not found: BaseURL`.
2. **`--background` cannot answer console prompts.** Any setting Tranquillity would ask for at first start must be in the configuration, or it dies with `Cannot see if a key has been pressed when ... console input has been redirected`. For the estate owner that includes `DefaultEstateOwnerUUID` (`OpenSimBase.SetUpEstateOwner` prompts for it when unset).
3. **Tranquillity bug: SQLite cannot create users.** At `713857f`, `UserAccountService` stores `DisplayName` and `NameChanged`, the MySQL migrations add those columns, and the SQLite `UserAccount.migrations` (version 3) does not. Creating any user on SQLite fails with `table UserAccounts has no column named DisplayName`. The harness adds the two columns after the first start, without changing the `migrations` version, so `ferroan-db`'s version check still sees what Tranquillity creates. The fix belongs in Tranquillity (a SQLite migration); report it there.
4. **Tranquillity bug: the HTTP server ignores its listen address.** `BaseHttpServer.Start` creates the listener with `IPAddress.Any` (`Source/OpenSim.Framework.Servers.HttpServer/BaseHttpServer.cs` line 2007) although the class has a `ListenIPAddress` property. The region's UDP port binds to `InternalAddress` (`127.0.0.1` above), but its HTTP port listens on every interface. The harness should carry a one-line scratch patch (like the capture patch, never sent upstream from here) so it binds to `127.0.0.1`; on a shared network, until then, keep the test account's password random and stop the region after use.
5. **Expected noise:** `Can't load experience service` (`Could not find a storage interface in the given module OpenSim.Data.SQLite.dll`). Tranquillity has no SQLite store for Experiences; the region runs without them.
6. Databases and the console history are written into the publish folder (`$P`), not into `$W/run`. A fresh run deletes `$P/*.db`.
