# Reference checkouts

Ferroan ports from, and checks against, these pinned revisions. The maintainers keep clean, read-only checkouts of them under `~/Git/reference/` (the paths below are relative to that folder). Never build in, edit or commit to a reference checkout; copy what you need into a worktree or `.scratch/`.

To move a pin, open a PR that changes this file and says why, and check what changed between the old and new revisions. For each bug in [TRANQUILLITY_BUGS.md](TRANQUILLITY_BUGS.md) fixed between the two revisions, regenerate the oracles and fixtures that touch it against the fixed behavior, change Ferroan to match, and move its row to Fixed.

| Checkout | Repository | Revision | Licence | Use |
| --- | --- | --- | --- | --- |
| `OpenSim-Tranquillity` | [OpenSim-NGC/OpenSim-Tranquillity](https://github.com/OpenSim-NGC/OpenSim-Tranquillity) `develop` | `1052cbae82ae7dc3b8abfcffe33e316498eeaad9` | BSD-3-Clause, MPL-2.0, Apache-2.0, MIT per file | The behavior reference; porting source; the server that captures and oracles run |
| `libopenmetaverse` | [OpenSim-NGC/libopenmetaverse](https://github.com/OpenSim-NGC/libopenmetaverse) | `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b` | BSD-3-Clause | The OpenMetaverse build Tranquillity ships (NuGet `UtopiaSkye.OpenMetaverse` 1.2.9). Porting source for types and server-side helpers; byte-level oracle for packets and LLSD |
| `Aurora-Viewer` | [Aurora-Viewer/Aurora-Viewer](https://github.com/Aurora-Viewer/Aurora-Viewer) `main` | `48cb76e830a13965ac080128bb874afedda86d69` | GPL-3.0-or-later | Rust crates to reuse (`aurora-llsd`, `aurora-msg`, `aurora-prim`); the viewer side of the protocol (`aurora-net`); the viewer used for login checks |
| `slua` | [secondlife/slua](https://github.com/secondlife/slua) | `a02d459aa0cb64a4c2b3dd4d841e67ba4a01d9b1` | MIT | Linden Lab's script engine (Phase 4, and the SLua work in Tranquillity) |
| `Ferrobot` | [BlackBox-Roleplay-Systems/Ferrobot](https://github.com/BlackBox-Roleplay-Systems/Ferrobot) `main` | `9cc7ab284c78b7f17ae209039e8a3405a12620b6` | GPL-3.0 | Second test client ([D17](DECISIONS.md)), built in scratch and run as a separate process through its signed HTTP API. See [LIVE.md](LIVE.md#ferrobot) for P0.10h commands and [TASKS.md](../TASKS.md) for validation. Not a behavior reference or porting source |
| `~/Git/viewer` (committed source only) | [secondlife/viewer](https://github.com/secondlife/viewer) | `ff75ab037607d8fe305f43f8b1f43126693ba2db` | LGPL-2.1 | Eight trailing message layouts from `scripts/messages/message_template.msg`, used under GPL-3.0-or-later as LGPL-2.1 section 3 permits. Message numbers, flags and existing blocks remain from libopenmetaverse |

Pin7 is [OpenSim-NGC/OpenSim-Tranquillity](https://github.com/OpenSim-NGC/OpenSim-Tranquillity) `develop` `1052cbae82ae7dc3b8abfcffe33e316498eeaad9` (2026-11-23), under [D21](DECISIONS.md): every PR Pin6 carried on the maintainer's `dev-build` has merged upstream, so the pin goes back to `develop`, as Pin4 did. Since `e28703a307` `develop` merged the maintainer's #473 to #491 and #500 to #502, John Legion's #492 to #498 (server-side baking tells a Hypergrid visitor by `LocalToGrid`, a faster Phlox test suite, a crossing back is not resent, Phlox `llGetWallclock` time zone, weather volumes for gods and their configuration, and the script-engine switch answers `NotSwitched`), #464 (an item-asset upload cap refuses an item of another type, and a task script update checks `CanEditScript`) and the groups series #441 to #452. The merged PRs have new commit hashes, so the trees were compared: `develop` differs from Pin6 `1d7abb8337a477842fe1d7dd84beffe389ee52ac` in 111 files, 86 of them tests. The groups series was already in Pin6 and adds no difference. Source changes outside tests: TB37, TB39, TB40, TB48, TB49 and TB50 are fixed (#488, #490, #491, #502, #500, #501); `Migration.Update` logs a failed step as a warning and still records it (#489, TB38 stays open); MySQL and PostgreSQL migration errors keep the database message; the region asset connector refuses a foreign asset whose id is not the one asked for (#464); `EntityTransferModule` and `ScenePresence` stop resending a crossing to an avatar that came back (#494); `osslDefaultEnable.ini` lets gods create and remove wind volumes; `RegionServer.ini.example` and `RegionServerDefaults.ini` document `[Phlox] GetWallclockTimeZone` as a comment. No migration file changed. The only open upstream PR from the maintainer is #499 (CI only). Pin6 was KanikaBreed/OpenSim-Tranquillity `dev-build` `1d7abb8337` (`develop` `e28703a307` plus #473 to #487). CI reads both repository URLs and revisions from this table. Set `FERROAN_REF` to the parent of clean alternate checkouts when needed.

Other checkouts the maintainers have, read for context only:

| Checkout | Repository | Use |
| --- | --- | --- |
| `~/Git/opensim` | [opensim/opensim](https://github.com/opensim/opensim) | Core OpenSimulator, for history and differences from Tranquillity |
| `~/Git/libremetaverse` | [cinderblocks/libremetaverse](https://github.com/cinderblocks/libremetaverse) | A newer OpenMetaverse fork (BSD-3-Clause); compare when libopenmetaverse is unclear |
| `~/Git/phoenix-firestorm` | [FirestormViewer/phoenix-firestorm](https://github.com/FirestormViewer/phoenix-firestorm) | Firestorm (LGPL-2.1): the viewer most residents use |
| `~/Git/dcs-srs-rust` | KanikaBreed/dcs-srs-rust | Workflow patterns: the check script, CI, and a C# oracle that verifies Rust-encoded fixtures |

## Toolchains on the maintainers' machine

- Rust stable through rustup.
- .NET SDK 10, for building Tranquillity at the reference revision and for the C# oracle.
- CMake and a C++ compiler, for Jolt and Linden's SLua engine in later phases.

The Second Life wiki's protocol pages refuse scripted downloads, so they are not mirrored here. Read them in a browser when needed; the code above is the specification Ferroan follows.
