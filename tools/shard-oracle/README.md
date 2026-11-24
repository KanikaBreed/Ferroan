# Shard-header oracle

A .NET 10 console program that calls `ServiceAuth.Create` and then `Handle` on the pinned authentication and grid-user post handlers. It does not start a Robust process. Build it against a scratch clone from `oracle_reference::prepare`, never against the reference checkout.

The committed capture is [fixtures/http/shard-header.json](../../fixtures/http/shard-header.json). Rust tests replay it. Nothing from the pin is copied into this repository.

```sh
dotnet build tools/shard-oracle /p:FerroanRef=.scratch/oracle-sources
dotnet run --project tools/shard-oracle --no-build -- <pin> fixtures/http/shard-header.json
```
