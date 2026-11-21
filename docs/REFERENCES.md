# Reference checkouts

Ferroan ports from, and checks against, these pinned revisions. The maintainers keep clean, read-only checkouts of them under `~/Git/reference/` (the paths below are relative to that folder). Never build in, edit or commit to a reference checkout; copy what you need into a worktree or `.scratch/`.

To move a pin, open a PR that changes this file and says why, and check what changed between the old and new revisions. For each bug in [TRANQUILLITY_BUGS.md](TRANQUILLITY_BUGS.md) fixed between the two revisions, regenerate the oracles and fixtures that touch it against the fixed behavior, change Ferroan to match, and move its row to Fixed.

| Checkout | Repository | Revision | Licence | Use |
| --- | --- | --- | --- | --- |
| `OpenSim-Tranquillity` | [KanikaBreed/OpenSim-Tranquillity](https://github.com/KanikaBreed/OpenSim-Tranquillity) `dev-build` | `1d7abb8337a477842fe1d7dd84beffe389ee52ac` | BSD-3-Clause, MPL-2.0, Apache-2.0, MIT per file | The behavior reference; porting source; the server that captures and oracles run |
| `libopenmetaverse` | [OpenSim-NGC/libopenmetaverse](https://github.com/OpenSim-NGC/libopenmetaverse) | `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b` | BSD-3-Clause | The OpenMetaverse build Tranquillity ships (NuGet `UtopiaSkye.OpenMetaverse` 1.2.9). Porting source for types and server-side helpers; byte-level oracle for packets and LLSD |
| `Aurora-Viewer` | [Aurora-Viewer/Aurora-Viewer](https://github.com/Aurora-Viewer/Aurora-Viewer) `main` | `48cb76e830a13965ac080128bb874afedda86d69` | GPL-3.0-or-later | Rust crates to reuse (`aurora-llsd`, `aurora-msg`, `aurora-prim`); the viewer side of the protocol (`aurora-net`); the viewer used for login checks |
| `slua` | [secondlife/slua](https://github.com/secondlife/slua) | `a02d459aa0cb64a4c2b3dd4d841e67ba4a01d9b1` | MIT | Linden Lab's script engine (Phase 4, and the SLua work in Tranquillity) |
| `Ferrobot` | [BlackBox-Roleplay-Systems/Ferrobot](https://github.com/BlackBox-Roleplay-Systems/Ferrobot) `main` | `9cc7ab284c78b7f17ae209039e8a3405a12620b6` | GPL-3.0 | Second test client ([D17](DECISIONS.md)), built in scratch and run as a separate process through its signed HTTP API. See [LIVE.md](LIVE.md#ferrobot) for P0.10h commands and [TASKS.md](../TASKS.md) for validation. Not a behavior reference or porting source |
| `~/Git/viewer` (committed source only) | [secondlife/viewer](https://github.com/secondlife/viewer) | `ff75ab037607d8fe305f43f8b1f43126693ba2db` | LGPL-2.1 | Eight trailing message layouts from `scripts/messages/message_template.msg`, used under GPL-3.0-or-later as LGPL-2.1 section 3 permits. Message numbers, flags and existing blocks remain from libopenmetaverse |

Pin6 is [KanikaBreed/OpenSim-Tranquillity](https://github.com/KanikaBreed/OpenSim-Tranquillity) `dev-build` `1d7abb8337a477842fe1d7dd84beffe389ee52ac` (2026-11-21), under [D21](DECISIONS.md). It is OpenSim-NGC `develop` `e28703a307` merged into the Pin5 roll-up, plus the maintainer's 15 PRs #473 (TB21, the PostgreSQL mute migration is embedded), #474 (TB34, `MuteListStore` version 2), #475 (TB27, MySQL XAsset store commits), #476 (TB28, MySQL XAsset `AssetsExist` reads `XAssetsMeta`), #477 (TB19, a full PostgreSQL asset metadata page), #478 (TB20, a refused asset write returns false), #479 (TB29, PostgreSQL XAsset metadata page), #480 (TB30, PostgreSQL XAsset delete), #481 (TB22, a failed FSAssets insert is an error), #482 (TB35, a second PostgreSQL FSAssets store succeeds), #483 (TB23, a missing or damaged FSAssets file is missing), #484 (TB25, `AssetsExist` checks the file), #485 (TB24, the writer retries), #486 (TB36, a failed spool write is not stored) and #487 (TB26, a visitor is told). `develop` `e28703a307` already carries #437, #438, #439, #440, #456, #462, #463, #465 and #466, so TB1 and TB10 to TB18 are fixed upstream. The tree differs from Pin5 `c70fee205f64ee8672a8d607cef6a733931da951` in 145 files. Pin5 was the same fork's `dev-build` at that revision. CI reads both repository URLs and revisions from this table. Set `FERROAN_REF` to the parent of clean alternate checkouts when needed.

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
