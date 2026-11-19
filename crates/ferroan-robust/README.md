# ferroan-robust

The grid-services process. This build serves the asset, user-account and user-alias routes whose ini sections are present. An ini with only `[AssetService]` still serves assets.

```sh
ferroan-robust [--inimaster FILE] [--inifile FILE] [--inidirectory DIR]
```

`--inimaster` defaults to `OpenSim.Server.GridServer.ini` in the working directory. Repeat `--inifile` for extra files. `--inidirectory` defaults to `config` and is skipped when that directory is absent. Loading follows `GridServerConfigSource` at Tranquillity `713857f10b8e91a639a967d4354a4682b080793c`.

The process listens on `[Network] bind_ip_address` and `[Network] port`; an empty bind address defaults to `0.0.0.0`. The live harness sets loopback explicitly. The asset section needs:

```ini
[AssetService]
    LocalServiceModule = "OpenSim.Services.AssetService.dll:AssetService"
    StorageProvider = "OpenSim.Data.SQLite.dll"
    ConnectionString = "Data Source=/path/to/AssetStore.db;Version=3"
```

`StorageProvider` and `ConnectionString` fall through to `[DatabaseService]` when the service section leaves them empty. An asset file must already be a Tranquillity SQLite database at AssetStore version 6. A user-account file must already be at UserAccount version 4. This program does not create either file and does not migrate it.

User accounts use the same shape:

```ini
[UserAccountService]
    LocalServiceModule = "OpenSim.Services.UserAccountService.dll:UserAccountService"
    StorageProvider = "OpenSim.Data.SQLite.dll"
    ConnectionString = "Data Source=/path/to/UserAccount.db;Version=3"
    AllowCreateUser = false
    AllowSetAccount = false
```

`AllowCreateUser` and `AllowSetAccount` default to false. `POST /accounts` is the user-account handler. `POST /useralias` is served when `[UserAliasService]` is present and the alias table already exists. The pin ships that table for MySQL only, so a SQLite alias section refuses to start until the table is created. MySQL and PostgreSQL account stores wait for P1.A1.

`DefaultAssetLoader` is ignored, so a fresh database does not gain the library asset set. Point a region's `[AssetService] AssetServerURI` at `http://127.0.0.1:<port>`. HTTPS and `AuthType = BasicHttpAuthentication` refuse to start.

The [Linux live workflow](../../.github/workflows/live.yml) points a C# region at this program during the asset comparison and persistent asset checks. The maintainer's Firestorm G0 evidence is recorded in [TASKS.md](../../TASKS.md). An Aurora viewer check is still pending.
