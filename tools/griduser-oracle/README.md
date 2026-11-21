# Grid user oracle

A .NET 10 console program that calls the pinned `SQLiteGridUserData`, `MySQLGridUserData` and `PGSQLGridUserData` plugins. Rust tests compare those rows. The program is a C# exception in [D13](../../docs/DECISIONS.md): it has to run Tranquillity's own types. Nothing from that checkout is copied into this repository.

`cargo xtask griduser cross` builds it. Build products go under `.scratch/griduser-oracle/`, not into the reference checkout. The command needs the .NET SDK, the pin checkout, and the local MySQL and PostgreSQL containers from `cargo xtask schema servers`. It does not start an HTTP server or the live harness. The Rust test skips with a printed reason unless that command sets `FERROAN_GRIDUSER_CROSS`.

Each invocation reads one JSON object on stdin and writes one JSON object on stdout. `FERROAN_GRIDUSER_BACKEND` is `sqlite`, `mysql` or `postgres`. `FERROAN_GRIDUSER_ADO` is the ADO connection string. `FERROAN_GRIDUSER_REALM` defaults to `GridUser`.

```sh
cargo xtask griduser cross
```

`get` returns `{found, user_id, data}` or `{found:false}`. `getall`, `getmany` and `online` return `{rows:[{user_id, data}]}`. `store` returns `{stored}`. `GetAll` on MySQL and PostgreSQL still interpolates the id, so a quote in the id makes that call fail. That is TB41. The Rust store binds an escaped prefix instead.
