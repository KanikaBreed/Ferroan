# Schema oracle

A .NET 10 program that creates SQLite databases by calling Tranquillity's `OpenSim.Data.Migration.Update`. It does not copy the migration SQL. The project references `OpenSim.Data.SQLite` in the pinned checkout (`FERROAN_REF`, default `~/Git/reference`).

This is the same kind of exception as `tools/reference-oracle`: it has to run Tranquillity's C# ([D13](../../docs/DECISIONS.md)). Build output goes to `.scratch/schema-oracle/`, not into the reference tree.

```sh
cargo xtask schema generate
cargo xtask schema verify
```

`generate` runs the migration twice, refuses to continue if the files differ, and writes `fixtures/db/sqlite/`. `verify` migrates again and compares those committed files byte for byte.

CI does not run this tool. The committed databases are what `ferroan-db`'s tests open. MySQL and PostgreSQL databases are not generated here; those checks run when `FERROAN_MYSQL_URL` or `FERROAN_POSTGRES_URL` is set.
