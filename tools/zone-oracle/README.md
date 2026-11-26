# Session-zone oracle

Run `cargo xtask session-zone` under the shared live lock with a free port base and `UseSharedCompilation=false`.

The command starts a disposable MySQL 8.4 server whose default zone is `-05:00` and a disposable PostgreSQL 16 server whose zone is `America/New_York`, then removes both. It does not use the schema-server containers and it does not start the live harness.

Pin8's MySQL and PostgreSQL authentication plugins read a token Ferroan stored, and the Presence plugins read a `LastSeen` Ferroan stored with `NOW()` or `now()`. A second pass sets Npgsql's `Timezone=Europe/Berlin` on the New York PostgreSQL server. A PostgreSQL login that cannot read the server zone stays on UTC unless the connection string sets `Timezone=` (D104).
