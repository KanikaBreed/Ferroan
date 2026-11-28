# OAR control oracle

A .NET 10 console program that loads an archive through the pinned `ArchiveReadRequest`. It writes each archive with `TarArchiveWriter` and, where the case says so, `CreateControlFile`. It then runs `FindAndLoadControlFile`, the `GetRegionFromPath` gate, and `DearchiveRegion(false)`. Rust tests compare that record. The program is a C# exception in [D13](../../docs/DECISIONS.md): it has to run Tranquillity's own types. Nothing from that checkout is copied into this repository.

`cargo xtask oar generate` builds it against a task-owned clone of the pin and writes [fixtures/oar-control](../../fixtures/oar-control/README.md). `cargo xtask oar verify` rebuilds it, writes the same files under `.scratch/oar-oracle/verify/`, and checks them against the committed fixture. Build products go under `.scratch/oar-oracle/`, not into the reference checkout. The commands need the .NET SDK and the pin checkout. They do not start a region. Ordinary tests replay the committed archives and do not start the oracle.

```sh
cargo xtask oar generate
cargo xtask oar verify
```

`generate <directory> <pin>` writes one gzip or tar per archive and `load.json`. The control file's datetime is the fixed stamp `1700000000`. A file load passes the console displacement, rotation and bounding options. A stream load passes an empty option map. The record is the control file and the region path. Archives in this fixture have no `objects/`, `assets/`, `terrains/`, `settings/` or `landdata/` members, so the later loaders are not entered.
