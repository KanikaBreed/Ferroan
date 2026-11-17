# Testing

## The local check

```sh
cargo xtask check              # links, formatting, clippy, tests
cargo xtask check --docs-only  # Markdown links only
```

The command is the same on every platform. CI runs it on Linux, Windows and macOS.

## The C# oracle

`cargo xtask oracle build`, `generate` and `verify` build [tools/reference-oracle](../tools/reference-oracle/README.md) against the pinned libopenmetaverse checkout and read or write [fixtures/oracle](../fixtures/oracle/README.md). `FERROAN_REF` selects the checkout root and defaults to `~/Git/reference`. The fixtures are committed. Packet fixtures are one datagram per message in the template; the layout is in [PROTOCOL.md](PROTOCOL.md). CI does not run the oracle; `cargo xtask oracle verify` is a manual step until a Linux CI job is added (P0.1 in [TASKS.md](../TASKS.md)). That command needs the .NET SDK 10. The reference checkout is only read; build output goes under `.scratch/`.

## Schema fixtures

`cargo xtask schema generate` and `verify` build [tools/schema-oracle](../tools/schema-oracle/README.md) and run Tranquillity's `Migration.Update` on SQLite. The databases are committed under [fixtures/db/sqlite](../fixtures/db/sqlite/README.md). CI does not run the tool. MySQL and PostgreSQL are checked by the same Rust code when `FERROAN_MYSQL_URL` or `FERROAN_POSTGRES_URL` is set; a CI job with those servers is not added yet (P0.7 in [TASKS.md](../TASKS.md)).

## Configuration fixtures

`cargo xtask config generate` and `verify` build [tools/config-oracle](../tools/config-oracle/README.md) and resolve INI files with Nini. The dumps are committed under [fixtures/config](../fixtures/config/README.md). CI does not run the tool. The Rust tests always check the small cases. The two cases that open Tranquillity's shipped INI files run when `FERROAN_REF` points at the checkout and are skipped otherwise (P0.6 in [TASKS.md](../TASKS.md)).

## Asset fixtures

`cargo xtask asset generate` and `verify` build [tools/asset-oracle](../tools/asset-oracle/README.md) and serialize `AssetBase` with Tranquillity's `ServerUtils.SerializeResult`. The documents are committed under [fixtures/asset](../fixtures/asset/README.md). CI does not run the tool. `ferroan-services` tests encode to those bytes and serve them from a copy of the AssetStore SQLite fixture. A Tranquillity region has not been pointed at `ferroan-robust`, and Aurora has not shown or uploaded a texture.

## Session captures

`ferroan-capture check` decodes every UDP record in a capture and requires the same bytes back. The steps, the Tranquillity patch and the redaction rules are in [CAPTURES.md](CAPTURES.md). CI runs the synthetic `UseCircuitCode` test and the local HTTP proxy test. It does not run Tranquillity, and no redacted session is committed yet. A raw capture is checked before redaction and is not committed. The committed file is the scrubbed encoding, checked again with `replay`.

## Evidence by kind of change

| Change | Evidence expected |
| --- | --- |
| Parsing or encoding (packets, LLSD, XML, archives) | Tests against bytes captured from Tranquillity, plus malformed and oversized input. A round trip through our own encoder and decoder alone is not enough |
| A grid service | Captured-traffic fixtures, then a run behind a test grid with C# regions |
| Region behavior | A viewer check (Aurora scripted, Firestorm by hand), recorded with viewer version and steps |
| Configuration | Resolved key/value sets from the Nini oracle, including the shipped region and grid examples when the reference checkout is present |
| Database access | Tests on SQLite databases created by Tranquillity's migration code. MySQL and PostgreSQL use the same check when `FERROAN_MYSQL_URL` or `FERROAN_POSTGRES_URL` is set |
| Native code (planned physics and scripting crates) | `SAFETY` comments on every unsafe block, and fuzzing of the boundary |

## Honest status

A compile check proves the code builds. It does not prove a viewer can log in, a script behaves as it did in C#, or a grid stays up. Record those as pending until someone has seen them work, with the revision, viewer or grid, and result.
