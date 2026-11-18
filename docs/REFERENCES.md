# Reference checkouts

Ferroan ports from, and checks against, these pinned revisions. The maintainers keep clean, read-only checkouts of them under `~/Git/reference/` (the paths below are relative to that folder). Never build in, edit or commit to a reference checkout; copy what you need into a worktree or `.scratch/`.

To move a pin, open a PR that changes this file and says why, and check what changed between the old and new revisions. For each bug in [TRANQUILLITY_BUGS.md](TRANQUILLITY_BUGS.md) fixed between the two revisions, regenerate the oracles and fixtures that touch it against the fixed behavior, change Ferroan to match, and move its row to Fixed.

| Checkout | Repository | Revision | Licence | Use |
| --- | --- | --- | --- | --- |
| `OpenSim-Tranquillity` | [OpenSim-NGC/OpenSim-Tranquillity](https://github.com/OpenSim-NGC/OpenSim-Tranquillity) `develop` | `858321883f5487a5d192d36ea9d7b90fa468310c` | BSD-3-Clause, MPL-2.0, Apache-2.0, MIT per file | The behavior reference; porting source; the server that captures and oracles run |
| `libopenmetaverse` | [OpenSim-NGC/libopenmetaverse](https://github.com/OpenSim-NGC/libopenmetaverse) | `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b` | BSD-3-Clause | The OpenMetaverse build Tranquillity ships (NuGet `UtopiaSkye.OpenMetaverse` 1.2.9). Porting source for types and server-side helpers; byte-level oracle for packets and LLSD |
| `Aurora-Viewer` | [Aurora-Viewer/Aurora-Viewer](https://github.com/Aurora-Viewer/Aurora-Viewer) `main` | `48cb76e830a13965ac080128bb874afedda86d69` | GPL-3.0-or-later | Rust crates to reuse (`aurora-llsd`, `aurora-msg`, `aurora-prim`); the viewer side of the protocol (`aurora-net`); the viewer used for login checks |
| `slua` | [secondlife/slua](https://github.com/secondlife/slua) | `a02d459aa0cb64a4c2b3dd4d841e67ba4a01d9b1` | MIT | Linden Lab's script engine (Phase 4, and the SLua work in Tranquillity) |
| `Ferrobot` | [BlackBox-Roleplay-Systems/Ferrobot](https://github.com/BlackBox-Roleplay-Systems/Ferrobot) `main` | `9cc7ab284c78b7f17ae209039e8a3405a12620b6` | GPL-3.0 | Planned second test client ([D17](DECISIONS.md)), run as a separate process against test regions. P0.10h harness integration is blocked and not shipped. Not a behavior reference and not a porting source |

Other checkouts the maintainers have, read for context only:

| Checkout | Repository | Use |
| --- | --- | --- |
| `~/Git/opensim` | [opensim/opensim](https://github.com/opensim/opensim) | Core OpenSimulator, for history and differences from Tranquillity |
| `~/Git/libremetaverse` | [cinderblocks/libremetaverse](https://github.com/cinderblocks/libremetaverse) | A newer OpenMetaverse fork (BSD-3-Clause); compare when libopenmetaverse is unclear |
| `~/Git/viewer` | [secondlife/viewer](https://github.com/secondlife/viewer) | Linden's viewer (LGPL-2.1): the message template's origin and viewer behavior |
| `~/Git/phoenix-firestorm` | [FirestormViewer/phoenix-firestorm](https://github.com/FirestormViewer/phoenix-firestorm) | Firestorm (LGPL-2.1): the viewer most residents use |
| `~/Git/dcs-srs-rust` | KanikaBreed/dcs-srs-rust | Workflow patterns: the check script, CI, and a C# oracle that verifies Rust-encoded fixtures |

## Toolchains on the maintainers' machine

- Rust stable through rustup.
- .NET SDK 10, for building Tranquillity at the reference revision and for the C# oracle.
- CMake and a C++ compiler, for Jolt and Linden's SLua engine in later phases.

The Second Life wiki's protocol pages refuse scripted downloads, so they are not mirrored here. Read them in a browser when needed; the code above is the specification Ferroan follows.
