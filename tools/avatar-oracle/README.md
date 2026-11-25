# Avatar oracle

A .NET 10 console program that calls the pinned `SQLiteAvatarData`, `MySQLAvatarData` and `PGSQLAvatarData` plugins. Rust tests compare those rows. The program is a C# exception in [D13](../../docs/DECISIONS.md): it has to run Tranquillity's own types. Nothing from that checkout is copied into this repository.

`cargo xtask avatar cross` builds it. Build products go under `.scratch/avatar-oracle/`, not into the reference checkout. The command needs the .NET SDK, the pin checkout, and the local MySQL and PostgreSQL containers from `cargo xtask schema servers`. It does not start an HTTP server or the live harness. The Rust test skips with a printed reason unless that command sets `FERROAN_AVATAR_CROSS`.

Each invocation reads one JSON object on stdin and writes one JSON object on stdout. `FERROAN_AVATAR_BACKEND` is `sqlite`, `mysql` or `postgres`. `FERROAN_AVATAR_ADO` is the ADO connection string. `FERROAN_AVATAR_REALM` defaults to `Avatars`.

```sh
cargo xtask avatar cross
```

`get` returns `{rows:[{principal_id, name, value}]}`. `upsert` returns `{stored}`. `delete` and `delete-principal` return `{deleted}`. The store writes the value it is given. Height parsing and the preserve-and-replace rules are the avatar service.

## Avatar service HTTP

`cargo xtask avatar service-generate` writes [the HTTP fixture](../../fixtures/avatar/service.json). `cargo xtask avatar service-verify` checks it against a newly migrated SQLite database. Both start Pin8's HTTP server on loopback at the private Robust port for `FERROAN_LIVE_PORT_BASE`. On a shared development host run them under the live lock. Each case records the connector form, status, content type and response text decoded as strict UTF-8, retaining every byte including any BOM. When a case names a refused key, the database adapter returns false for that store only. Subsequent reads go through the real handler and SQLite plugin. The height table uses a dot-decimal process culture. With `FERROAN_AVATAR_ADO_MYSQL` set to a disposable test database, the commands also record or check [five MySQL replies](../../fixtures/avatar/service-mysql.json). They copy the migrated `Avatars` schema to a private table, submit a 33-character name after a serial update, read the remaining bake data, then drop the copy to make an item delete fail during execution. The Rust MySQL test uses `FERROAN_MYSQL_URL` and a separate copy.
