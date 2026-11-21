# Asset oracle

A .NET 10 console program that serializes `AssetBase`, `AssetMetadata` and the small XML values the asset handlers return, using Tranquillity `ServerUtils.SerializeResult`. Rust tests compare those bytes. The program is a C# exception in [D13](../../docs/DECISIONS.md): it has to run Tranquillity's own types. Nothing from that checkout is copied into this repository.

`cargo xtask asset` builds it. Build products go under `.scratch/asset-oracle/`, not into the reference checkout.

```sh
cargo xtask asset generate
cargo xtask asset verify
```

`FERROAN_REF` is the reference checkouts root. It defaults to `~/Git/reference`. The checkout's OpenSim-Tranquillity revision must match `docs/REFERENCES.md`, and the checkout must be clean before and after the build. The [Linux live workflow](../../.github/workflows/live.yml) runs `cargo xtask asset verify`; ordinary CI runs Rust tests against the committed fixtures. `unnamed-flags.json` records `StoredAssetFlags.FromStored` masking flags 8. It is not one of the serialized documents.

`cargo xtask asset cross` builds the same program and calls `SQLiteAssetData`, `MySQLAssetData` and `PGSQLAssetData` (`AssetOracle db`). It needs the .NET SDK, the pin checkout, and the local MySQL and PostgreSQL containers from `cargo xtask schema servers`. The Rust test skips with a printed reason unless that command sets `FERROAN_ASSET_CROSS`.
