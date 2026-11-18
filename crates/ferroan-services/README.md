# ferroan-services

The asset service a Tranquillity region calls on Robust.

The routes follow `Source/OpenSim.Server.Handlers/Asset/` at Tranquillity `713857f10b8e91a639a967d4354a4682b080793c`:

| Method and path | What it does | Content type |
| --- | --- | --- |
| `GET /assets/{id}` | `AssetBase` XML, or 404 | `text/xml` |
| `GET /assets/{id}/data` | raw bytes, or 404 | `application/octet-stream` |
| `GET /assets/{id}/metadata` | `AssetMetadata` XML, or 404 | the asset type's content type |
| `POST /assets` and `POST /assets/` | store, and return the id string | `application/xml` |
| `POST /assets/{id}` | `UpdateContent`, which returns false | `application/xml` |
| `DELETE /assets/{id}` | delete when the config allows it, and return a bool | `application/xml` |
| `POST /get_assets_exist` | one bool per id | `application/xml` |

`POST`, `DELETE` and exists keep the stream-handler default `application/xml`. GET of the whole asset sets `text/xml`. A missing asset is 404 with an empty `text/plain` body. `RedirectURL`, when set, turns that 404 into 302. The location is the configured text plus the request path, including a double slash when the configured text already ends in `/`.

XML bytes are `ServerUtils.SerializeResult`: UTF-8 BOM, LF, two-space indentation, no newline after the root. The comparisons are `fixtures/asset`, written by `cargo xtask asset`. Flag text is the `[Flags]` names (`Normal`, `Maptile Rewritable`). A value with a bit outside `Normal`, `Maptile`, `Rewritable` and `Collectable` makes `XmlSerializer` throw; Ferroan writes the decimal number so the row can still be fetched. `SHA1` is omitted. `CreationDate` is `0001-01-01T00:00:00`.

## Store

SQLite only, on a file Tranquillity already migrated to AssetStore version 6. This crate does not create the schema and does not run migrations. MySQL asset SQL is not ported.

`AssetService.Store` skips the write when the id exists, so a second store does not change the row. `SQLiteAssetData.StoreAsset` would update it; the service never calls that branch for an existing id. Names and descriptions are cut to 64 UTF-16 code units on insert. A non-BMP character that would be split in half by `Substring` is dropped instead, because a Rust string cannot hold a lone surrogate. Existence checks are parameterised. The C# method concatenates ids after `UUID.Parse`. One id that does not parse makes the whole answer false.

`UpdateContent` returns false. Delete is off unless `AllowRemoteDelete` is true, and then only maptiles (`Flags` bit 0) unless `AllowRemoteDeleteAllTypes` is also true. A parsed id answers true after the delete statement even when the row was already gone.

A store document with no `Data` element is 400. C# would insert NULL, and the later get would throw. Region connectors send the bytes.

## Not in this pilot

The filesystem default-asset loader, FSAssets (`XAssetService`), the `show asset` / `delete asset` / `dump asset` console commands, and `FetchAssetMetadataSet`. `AuthType = BasicHttpAuthentication` and `https_main = true` refuse to start. `TrustedGridAuthentication` does not reject when no trust context is set, which is this process. `X-SecondLife-Shard` is 403 on get, post and delete unless `AllowllHTTPRequestIn` is true. Exists does not check that header.

Bodies larger than 50_000_000 bytes are refused. Tranquillity does not cap `XmlSerializer`.

Ordinary CI compares the committed asset XML and SQLite fixtures in Rust tests. The [Linux live workflow](../../.github/workflows/live.yml) runs the C# asset oracle, compares texture and inventory traffic against both services, and checks a persistent C# region using `ferroan-robust`. The maintainer's Firestorm asset check is recorded under G0 in [TASKS.md](../../TASKS.md).
