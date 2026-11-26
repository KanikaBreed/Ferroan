# Null Presence observations

`null.json` records the calls and replies from the Pin8 `OpenSim.Data.Null.NullPresenceData` plugin. The fixture uses synthetic IDs and two plugin instances in one process. A `get` retains the returned object; `held` reads that same reference again after a report or a replacement. A `restore` stores the retained object through the other instance. Multi-row results keep the provider's enumeration order.

Run `cargo xtask presence generate` to regenerate the fixture, or `cargo xtask presence verify` to compare the complete output with the committed file. The [oracle](../../tools/presence-oracle/Program.cs) uses the revision in [REFERENCES.md](../../docs/REFERENCES.md).
