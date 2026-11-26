# Presence oracle

`cargo xtask presence generate` writes `fixtures/presence/null.json`.
`cargo xtask presence verify` repeats the calls and compares the complete file byte for byte.

The oracle constructs two instances of the pinned `OpenSim.Data.Null.NullPresenceData` provider in one process. It records the inputs and results of all seven store operations, including field lookup order and what a retained row contains after a region report or session replacement. The Rust test `two_null_providers_match_the_pin8_sequence` replays those calls through the shared Presence store interface. [Fixture](../../fixtures/presence/README.md)
