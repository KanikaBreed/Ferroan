# Session-zone oracle

Run `cargo xtask session-zone` under the shared live lock with a free port base and `UseSharedCompilation=false`.

The command starts a disposable MySQL 8.4 server whose default zone is `-05:00` and a disposable PostgreSQL 16 server whose zone is `America/New_York`, then removes both. It does not use the schema-server containers and it does not start the live harness.

Pin8's MySQL and PostgreSQL authentication plugins read a token Ferroan stored, and the Presence plugins read a `LastSeen` Ferroan stored with `NOW()` or `now()`. A second pass sets Npgsql's `Timezone=Europe/Berlin` on the New York PostgreSQL server. A PostgreSQL login that cannot read the server zone stays on UTC unless the connection string sets `Timezone=` or the process has `PGTZ` within `db.timezone` (D104). `Timezone=` and `PGTZ` are sent as given, including a fixed offset or a POSIX zone. `ALTER DATABASE` and `ALTER ROLE` zones are applied before the command line and the configuration file. A role named `OpenSim` keeps its `ALTER ROLE` zone after `REVOKE SELECT` on `pg_database` and `pg_roles`, and after `REVOKE EXECUTE` on `pg_stat_get_activity(integer)` when `pg_database` is readable again. A login without `SELECT` on `pg_db_role_setting` still connects.
