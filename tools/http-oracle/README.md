# HTTP oracle

A small C# program that calls Tranquillity `ServerUtils.ParseQueryString`, `BuildQueryString`, `BuildXmlResponse` and `ParseXmlResponse`, and writes what those methods return.

`ServerUtils.cs` is not copied into this repository. The program references `OpenSim.Server.Base` in the read-only checkout named by `FERROAN_REF` (default `~/Git/reference`). Build output goes under the Ferroan worktree's `.scratch/http-oracle/`, via `UseArtifactsOutput`.

```sh
cargo xtask http generate
cargo xtask http verify
```

The [Linux live workflow](../../.github/workflows/live.yml) runs `cargo xtask http verify`. Ordinary CI runs the Rust tests comparing `ferroan-http` with the committed files.
