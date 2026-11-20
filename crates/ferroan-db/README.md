# ferroan-db

sqlx pools for the three databases Tranquillity ships: SQLite, MySQL (MySqlConnector 2.6.2) and PostgreSQL (Npgsql 10.0.3).

`Pool::open` reads the `migrations` table and returns an error when a requested store is not at the version Tranquillity's migration resources reach from an empty database. The expected numbers are in `src/expected.rs`. They are not the same on each backend. The crate does not run migrations and does not create a missing SQLite file.

The versions come from `Source/OpenSim.Data/Migration.cs` and the `Resources/*.migrations` files (plus SQLite's old-style `001_GridUserStore.sql`) in the pinned Tranquillity checkout `c70fee205f64ee8672a8d607cef6a733931da951`. Pin5 adds SQLite `UserAccount` version 5 (`UserCountry`) and a new SQLite `os_groups_Store` version 2, and raises MySQL and PostgreSQL `UserAccount` and `os_groups_Store`. A test re-reads that checkout when `FERROAN_REF` (default `~/Git/reference`) is present and its `HEAD` is that revision. A checkout at another revision is skipped, so a stale local reference does not fail the suite. Ordinary CI has no checkout and uses the committed table and SQLite files under `fixtures/db/sqlite/`. The [Linux live workflow](../../.github/workflows/live.yml) has the pinned checkout and runs `cargo xtask schema verify` against C#; it does not rerun this Rust catalog-source test.

## Connection strings

The parser accepts the forms in Tranquillity's shipped INI examples.

- SQLite: `URI=file:Asset.db` and `URI=file:OpenSim.db;UseUTF16Encoding=True`. The file must already exist. `UseUTF16Encoding` is accepted and ignored: the connection is UTF-8, and SQLite converts stored text on read. `mode=rwc` does not create the file.
- MySQL: `Data Source=localhost;Database=opensim;User ID=opensim;Password=...;SslMode=None`. With no `SslMode`, the mode is `Preferred`, which is MySqlConnector 2.6's default.
- PostgreSQL: `Server=localhost;Database=opensim;User Id=opensim;Password=...;SSL Mode=Disable`. With no SSL mode, the mode is `Prefer`, which is Npgsql 10's default.

`Maximum Pool Size` is rejected. Every pool is capped at 8 connections.

## What is not checked here

MySQL and PostgreSQL tests run only when `FERROAN_MYSQL_URL` or `FERROAN_POSTGRES_URL` is set to a sqlx URL for a database Tranquillity has migrated. No such server was available for the first version of this crate. A Linux CI job with service containers is still to be added.

SQLite is linked as bundled `libsqlite3-sys` ([D14](../../docs/DECISIONS.md)). Building this crate needs a C compiler.
