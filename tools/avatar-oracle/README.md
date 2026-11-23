# Avatar oracle

A .NET 10 console program that calls the pinned `SQLiteAvatarData`, `MySQLAvatarData` and `PGSQLAvatarData` plugins. Rust tests compare those rows. The program is a C# exception in [D13](../../docs/DECISIONS.md): it has to run Tranquillity's own types. Nothing from that checkout is copied into this repository.

`cargo xtask avatar cross` builds it. Build products go under `.scratch/avatar-oracle/`, not into the reference checkout. The command needs the .NET SDK, the pin checkout, and the local MySQL and PostgreSQL containers from `cargo xtask schema servers`. It does not start an HTTP server or the live harness. The Rust test skips with a printed reason unless that command sets `FERROAN_AVATAR_CROSS`.

Each invocation reads one JSON object on stdin and writes one JSON object on stdout. `FERROAN_AVATAR_BACKEND` is `sqlite`, `mysql` or `postgres`. `FERROAN_AVATAR_ADO` is the ADO connection string. `FERROAN_AVATAR_REALM` defaults to `Avatars`.

```sh
cargo xtask avatar cross
```

`get` returns `{rows:[{principal_id, name, value}]}`. `upsert` returns `{stored}`. `delete` and `delete-principal` return `{deleted}`. The store writes the value it is given. Height parsing and the preserve-and-replace rules are the avatar service.
