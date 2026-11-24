# Region store oracle

A .NET 10 console program that loads one region's prims from a SQLite region store through the pinned region store. Rust tests compare those groups. The program is a C# exception in [D13](../../docs/DECISIONS.md): it has to run Tranquillity's own types. Nothing from that checkout is copied into this repository.

`cargo xtask regionstore generate` builds it against a task-owned clone of the pin and writes [fixtures/regionstore/objects.db](../../fixtures/regionstore/objects.db) and [objects.json](../../fixtures/regionstore/objects.json). `cargo xtask regionstore verify` rebuilds it and reloads that file. Build products go under `.scratch/regionstore-oracle/`, not into the reference checkout. The commands need the .NET SDK and the pin checkout. They do not start a server or the live harness. Ordinary tests replay the committed dump and do not start the oracle.

```sh
cargo xtask regionstore generate
cargo xtask regionstore verify
```

`seed <db>` creates the file and inserts the fixture rows. `load <db> <region-uuid> <pin>` writes one JSON object to stdout and records that pin. A row that cannot be built is absent. Texture entries and extra params of a stored shape stay the stored bytes. The eight structured payloads are dumped both as the stored cell and as the value the load produced. An animation sequence number is a clock and is absent. Inventory items are a later row.
