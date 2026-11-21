# MySQL schema fixture

`schema.sql` and `manifest.txt` are a database Tranquillity's `Migration.Update` produced at the revision in the manifest. `cargo xtask schema servers` runs `tools/schema-oracle` against the local `ferroan-q20-mysql` container. The MySQL asset plugin uses the base `Migration` class, and this dump does too, for every embedded store.

The Linux CI job loads `schema.sql` into a throwaway MySQL 8.4 container. Do not edit the file by hand. Regenerating it needs the .NET SDK, `FERROAN_REF` (default `~/Git/reference`), and that container.
