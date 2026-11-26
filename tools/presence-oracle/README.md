# Presence oracle

## PostgreSQL

Run `cargo xtask presence cross postgres` under the shared live lock with a free port base and `UseSharedCompilation=false`. Set `FERROAN_PRESENCE_ADO_POSTGRES` in the environment to a disposable PostgreSQL database, for example `Host=127.0.0.1;Port=5432;Database=presence_test;Username=presence_test`. If authentication requires a password, append the `Password` field to the environment value. Omit `Pooling` and `Maximum Pool Size` because ferroan-db refuses both keys. Both providers use that connection and repeat the sequence in a separately named realm with the same indexes and defaults. The Pin8 plugin runs its migrations, so the test account needs permission to create the schema.

C# and Rust insert, update and read one another's sessions. The plugin checks retained and restored values, and a native read checks SQL NULL and every fractional digit of the reported timestamp. The cross-read also records the Pin8 re-store exception for fetched NULL and reported timestamps (TB105) and its timestamp selector exceptions (TB114).

## Null

`cargo xtask presence generate` writes `fixtures/presence/null.json`.
`cargo xtask presence verify` repeats the calls and compares the complete file byte for byte.

The oracle constructs two instances of the pinned `OpenSim.Data.Null.NullPresenceData` provider in one process. It records the inputs and results of all seven store operations, including field lookup order and what a retained row contains after a region report or session replacement. The Rust test `two_null_providers_match_the_pin8_sequence` replays those calls through the shared Presence store interface. [Fixture](../../fixtures/presence/README.md)

`cargo xtask presence cross mysql` runs the Pin8 MySQL plugin and the Rust provider against one disposable MySQL 8.4 database. Set `FERROAN_PRESENCE_ADO_MYSQL` to its ADO connection string. The account needs permission to run the Presence migration on an empty database. Credentials stay in the environment. Each run uses its own session, user and region IDs. It repeats the sequence in a second realm with a copy of the migration schema. The C# read checks the stored Unix second and its rendering in the server session time zone.

The C# plugin writes and replaces sessions, moves one to another region and tries to store the fetched row. Rust checks those rows, stores a fetched timestamp, replaces a session, reports its region, and logs it out. The plugin checks the Rust writes and logout. TB104 and D84 account for the difference on re-store.
