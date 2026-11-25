# Presence oracle

`cargo xtask presence generate` writes `fixtures/presence/null.json`.
`cargo xtask presence verify` repeats the calls and compares the complete file byte for byte.

The oracle constructs two instances of the pinned `OpenSim.Data.Null.NullPresenceData` provider in one process. It records the inputs and results of all seven store operations, including field lookup order and what a retained row contains after a region report or session replacement. The Rust test `two_null_providers_match_the_pin8_sequence` replays those calls through the shared Presence store interface. [Fixture](../../fixtures/presence/README.md)

`cargo xtask presence cross mysql` runs the Pin8 MySQL plugin and the Rust provider against one disposable MySQL 8.4 database. Set `FERROAN_PRESENCE_ADO_MYSQL` to its ADO connection string. The account needs permission to run the Presence migration on an empty database. Credentials stay in the environment. Each run uses its own session, user and region IDs.

The C# plugin writes and replaces sessions, moves one to another region and tries to store the fetched row. Rust checks those rows, stores a fetched timestamp, replaces a session, reports its region, and logs it out. The plugin checks the Rust writes and logout. TB104 and proposed D84 account for the difference on re-store.
