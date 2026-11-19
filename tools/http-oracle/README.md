# HTTP oracle

A small C# program that calls Tranquillity `ServerUtils.ParseQueryString`, `BuildQueryString`, `BuildXmlResponse` and `ParseXmlResponse`, and writes what those methods return.

`ServerUtils.cs` is not copied into this repository. The program references `OpenSim.Server.Base` in the read-only checkout named by `FERROAN_REF` (default `~/Git/reference`). Build output goes under the Ferroan worktree's `.scratch/http-oracle/`, via `UseArtifactsOutput`.

```sh
cargo xtask http generate
cargo xtask http verify
cargo xtask http status-generate
cargo xtask http status-verify
```

The [Linux live workflow](../../.github/workflows/live.yml) runs `cargo xtask http verify` and `cargo xtask http status-verify`. Ordinary CI runs the Rust tests comparing `ferroan-http` with the committed files.

`status-generate` and `status-verify` call the pinned Grid, Presence and GridUser connector methods against a loopback responder. The responder injects HTTP 200, 201, 302 without a `Location`, and 500, each with valid-looking success XML. Each case runs twice and must produce the same request and connector result. The build uses the task-owned clone under `.scratch/oracle-sources`, the same clone as `http verify`, and does not write into the read-only reference checkout. The fixtures are [fixtures/http-status](../../fixtures/http-status/README.md).
