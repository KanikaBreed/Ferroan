# Asset oracle

A .NET 10 console program that serializes `AssetBase`, `AssetMetadata` and the small XML values the asset handlers return, using Tranquillity `ServerUtils.SerializeResult`. Rust tests compare those bytes. The program is a C# exception in [D13](../../docs/DECISIONS.md): it has to run Tranquillity's own types. Nothing from that checkout is copied into this repository.

`cargo xtask asset` builds it. Build products go under `.scratch/asset-oracle/`, not into the reference checkout.

```sh
cargo xtask asset generate
cargo xtask asset verify
```

`FERROAN_REF` is the reference checkouts root. It defaults to `~/Git/reference`. The checkout's OpenSim-Tranquillity revision must match `docs/REFERENCES.md`, and the checkout must be clean before and after the build. The [Linux live workflow](../../.github/workflows/live.yml) runs `cargo xtask asset verify`; ordinary CI runs Rust tests against the committed fixtures. `unnamed-flags.json` records `StoredAssetFlags.FromStored` masking flags 8, and `asset/unnamed-flags.xml` is the GET body. `mixed-flags.json` records flags 9 (Maptile mixed with bit 8), and `asset/mixed-flags.xml` is that GET body. The JSON files are not manifest cases.

`generate` and `verify` also call `AssetLoaderFileSystem.ForEachDefaultXmlAsset` and the loader's `StoreAsset` callback, and they record `show asset`, `dump asset` and `delete asset`. After the C# program writes `loader-sample.json`, `seed.json`, `commands.json` and `library-manifest.json`, both commands run `cargo test -p ferroan-services --lib pilot_`. The library manifest is metadata. The asset bytes stay in the reference checkout.

`cargo xtask asset cross` builds the same program and calls `SQLiteAssetData`, `MySQLAssetData` and `PGSQLAssetData` (`AssetOracle db`). It needs the .NET SDK, the pin checkout, and the local MySQL and PostgreSQL containers from `cargo xtask schema servers`. The Rust test skips with a printed reason unless that command sets `FERROAN_ASSET_CROSS`.

`cargo xtask asset xasset-cross` calls `MySQLXAssetData` and `PGSQLXAssetData` (`AssetOracle xdb`) the same way. The Rust test skips unless that command sets `FERROAN_XASSET_CROSS`. It does not start an HTTP server or the live harness. `XAssetService` over HTTP is the Rust service tests and `cargo xtask live assets --backend xasset --db mysql|pgsql`, which lists TB27, TB28, TB29, TB30 and TB33 before `assets: match`. TB30 on that list is a meta-only delete: the data row stays, and the pin still leaves the meta row.

`cargo xtask asset fsassets-cross` calls `MySQLFSAssetData` and `PGSQLFSAssetData` (`AssetOracle fdb`) the same way. The Rust test skips unless that command sets `FERROAN_FSASSET_CROSS`. It does not start an HTTP server, write asset files, or run the live harness. The cross-read lists TB22 on MySQL and TB35 and TB39 on PostgreSQL. `live assets --backend fsassets` stays refused until Q23e.

`cargo xtask asset fsassets-files` calls the pin's `HashToPath`, `HashToFile` and `GetFsData` (`AssetOracle ffile`) without constructing `FSAssetConnector`, so it does not open a database or start the writer. Rust and C# read each other's files. The path and the decompressed bytes must match. Compressed bytes need not match. The Rust test skips unless that command sets `FERROAN_FSASSET_FILES`. A damaged or missing file is missing on both sides (`ffile checked` calls the pin's `GetCheckedFsData`). It does not start an HTTP server or the live harness.
