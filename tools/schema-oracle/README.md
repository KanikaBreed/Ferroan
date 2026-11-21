# Schema oracle

A .NET 10 program that creates SQLite databases by calling Tranquillity's `OpenSim.Data.Migration.Update`. It does not copy the migration SQL. The project references `OpenSim.Data.SQLite` in the pinned checkout (`FERROAN_REF`, default `~/Git/reference`).

This is the same kind of exception as `tools/reference-oracle`: it has to run Tranquillity's C# ([D13](../../docs/DECISIONS.md)). Build output goes to `.scratch/schema-oracle/`, not into the reference tree.

```sh
cargo xtask schema generate
cargo xtask schema verify
```

`generate` runs the migration twice, refuses to continue if the files differ, and writes `fixtures/db/sqlite/`. `verify` migrates again and compares those committed files byte for byte.

The [Linux live workflow](../../.github/workflows/live.yml) runs `cargo xtask schema verify`. The committed SQLite databases are what `ferroan-db`'s tests open on every platform.

```sh
cargo xtask schema servers
```

`servers` resets the local `ferroan-q20-mysql` (port 13306) and `ferroan-q20-postgres` (port 15432) containers, runs `Migration.Update` for every embedded store, and writes `fixtures/db/mysql` and `fixtures/db/postgres`. MySQL uses the base `Migration` class. PostgreSQL uses `PGSQLMigration`. `MuteListStore.migrations` is not embedded in the PGSQL project (TB21); when that row is missing, the oracle applies the file from its own assembly. Do not edit the dumps by hand. The Linux CI job loads them. It does not run this command: ordinary CI has no .NET SDK and no reference checkout.
