# PostgreSQL schema fixture

`schema.sql` and `manifest.txt` are a database Tranquillity's `Migration.Update` produced at the revision in the manifest. `cargo xtask schema servers` runs `tools/schema-oracle` against the local `ferroan-q20-postgres` container. PostgreSQL stores use `PGSQLMigration`, which is what `PGSQLAssetData` uses through `CheckMigration`.

`MuteListStore.migrations` is not embedded in the PGSQL project (TB21). When that row is missing, the oracle applies the file and the dump includes version 1. A C# server that never embedded the file does not create the row.

The Linux CI job loads `schema.sql` into a throwaway PostgreSQL 16 container. Do not edit the file by hand. Regenerating it needs the .NET SDK, `FERROAN_REF` (default `~/Git/reference`), and that container. `pg_dump`'s random `\restrict` line is removed so the file stays stable.
