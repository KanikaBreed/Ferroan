# SQLite schema fixtures

These databases were created by Tranquillity's `OpenSim.Data.Migration.Update` at the revision in `manifest.txt`. `cargo xtask schema generate` runs `tools/schema-oracle`, which references the pinned checkout and does not copy its SQL into this repository.

Each file is one store, the way a standalone region's SQLite config uses one file per service. The `migrations` table has the meta row `migrations` at version 1 and one row for that store. Ferroan's tests open the files and refuse a copy whose version was changed.

Do not edit the files by hand. Regenerate them with `cargo xtask schema generate`. That needs the .NET SDK and `FERROAN_REF` (default `~/Git/reference`).
