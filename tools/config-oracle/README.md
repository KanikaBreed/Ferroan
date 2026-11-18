# Config oracle

A .NET 10 program that resolves INI files by calling Nini 0.9.2.16 and `OpenSim.Framework.Util.Glob`. It does not copy either body of code. The project references `OpenSim.Framework` in the pinned checkout (`FERROAN_REF`, default `~/Git/reference`) and the NuGet package `nini-core` 0.9.2.16.

This is the same kind of exception as `tools/reference-oracle`: it has to run Tranquillity's C# and the Nini package Tranquillity references ([D13](../../docs/DECISIONS.md)). Build output goes to `.scratch/config-oracle/`, not into the reference tree.

```sh
cargo xtask config generate
cargo xtask config verify
```

`generate` runs every case twice, refuses to continue if the two runs differ, and writes `fixtures/config/`. `verify` resolves again and compares those committed files byte for byte.

The region and grid sequences follow `ConfigurationLoader.cs` and `GridServerConfigSource.cs` at the Tranquillity revision in [docs/REFERENCES.md](../../docs/REFERENCES.md), also recorded in the generated manifest. The region directory filter here uses `ToLowerInvariant`. Tranquillity uses the current culture's `ToLower()`. The shipped names are ASCII, so the two agree.

The [Linux live workflow](../../.github/workflows/live.yml) runs `cargo xtask config verify`, including the shipped INI cases. Ordinary CI runs `ferroan-config`'s tests against the committed fixtures; its two Rust cases that require the reference checkout are skipped there. Run the Rust tests locally with the pinned reference present to include those comparisons.
