# Presence oracle

## PostgreSQL

Run `cargo xtask presence cross postgres` under the shared live lock with a free port base and `UseSharedCompilation=false`. Set `FERROAN_PRESENCE_ADO_POSTGRES` in the environment to a disposable PostgreSQL database. Both providers use that connection and repeat the sequence in a separately named realm with the same indexes and defaults. The Pin8 plugin runs its migrations, so the test account needs permission to create the schema.

C# and Rust insert, update and read one another's sessions. The plugin checks retained and restored values, and a native read checks SQL NULL and every fractional digit of the reported timestamp. The cross-read also records the Pin8 re-store exception (TB105) and its timestamp selector exceptions (TB114).

## Null

`cargo xtask presence generate` writes `fixtures/presence/null.json`.
`cargo xtask presence verify` repeats the calls and compares the complete file byte for byte.

The oracle constructs two instances of the pinned `OpenSim.Data.Null.NullPresenceData` provider in one process. It records the inputs and results of all seven store operations, including field lookup order and what a retained row contains after a region report or session replacement. The Rust test `two_null_providers_match_the_pin8_sequence` replays those calls through the shared Presence store interface. [Fixture](../../fixtures/presence/README.md)
