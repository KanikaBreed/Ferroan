# Decisions

Settled decisions. To change one, open a PR that edits this file and says why.

| # | Decision | Chosen | Notes |
| --- | --- | --- | --- |
| D1 | Do the SLua engine plan in today's C# server first? | Yes. Run Linden's SLua engine in Tranquillity, and keep using C# until Ferroan is stable | Phase 4 reuses that work's C interface and conformance suite |
| D2 | Mixed grids or one big switch | Dual support until the switch: Rust and C# pieces run on the same grid | Every phase keeps wire compatibility with the C# server |
| D3 | Where the code lives | This repository, run by the team | Private for now |
| D4 | Licence | GPL-3.0-or-later for the project; each ported or copied file keeps its original notices and references | Changed from MPL-2.0 on 17 November 2026 so Aurora Viewer's Rust code can be reused. BSD, MPL-2.0, Apache-2.0, MIT and LGPL-2.1 sources may all be combined into a GPL-3.0 work. Ferroan code cannot flow back into Tranquillity's BSD/MPL tree ([PORTING.md](PORTING.md)) |
| D5 | Team | Volunteers | Calendar time depends on hours given ([PLAN.md](PLAN.md)) |
| D6 | Async runtime and HTTP stack | tokio for networking and services; axum and hyper for HTTP servers, reqwest as client. The region frame loop, Jolt and the SLua engine run on their own threads | tokio is the most widely used runtime, and axum, hyper, reqwest and sqlx build on it. A frame loop needs a fixed tick, and both native libraries expect a thread of their own |
| D7 | Feature freeze in C# | Freeze each part's features once its Rust phase starts | Bug fixes still go into both |
| D8 | Native libraries | Only Jolt and Linden Lab's SLua engine | A third needs its own decision here |
| D9 | Physics engines | Jolt only | BulletSim, ubODE, POS and basic physics are not ported |
| D10 | One script engine | Linden's SLua engine runs LSL, SLua, OSSL and bot functions | Phlox's VM and YEngine are not ported; YEngine's language extensions are unsupported |
| D11 | Reusing Aurora Viewer | Reuse Aurora's Rust crates where the code is the same on both ends: LLSD, message template code generation, prim volumes | Copied files keep Aurora's notices and record the Aurora revision. Viewer-side logic (circuit, session, UI) is read as a reference only, since the server's side differs |
| D12 | Where code comes from | Rust code Aurora already has (D11) comes from Aurora. C# code Aurora lacks is ported from OpenSim-NGC/libopenmetaverse, the exact OpenMetaverse build Tranquillity uses, or from Tranquillity itself | libopenmetaverse and Tranquillity are also the byte-level oracles for every fixture. Revisions are pinned in [REFERENCES.md](REFERENCES.md) |
| D13 | Rust first | All Ferroan code is Rust, including tools, scripts, test helpers and CI tasks (`cargo xtask`). Another language needs an entry here with the reason | Allowed now: the C# oracle (`tools/reference-oracle`), the SQLite schema fixture tool (`tools/schema-oracle`, it must run Tranquillity's `Migration` class), the INI oracle (`tools/config-oracle`, it must call Nini and Tranquillity's include walk) and the Tranquillity capture patch, because they must run or change C# code; Jolt and the SLua engine (D8) |
| D14 | SQLite inside sqlx | `ferroan-db` links `libsqlite3-sys` with SQLite bundled, so the SQLite driver builds on Linux, Windows and macOS without a system package. MySQL and PostgreSQL use sqlx's Rust protocol code and add no native library | This is the database driver named in D6. It does not loosen D8: Jolt and SLua remain the only native libraries in the physics and scripting crates, and a further native library still needs its own decision |

D1 to D7, D11 and D13 were settled on 17 November 2026 (D4 revised the same day). D8 to D10 come from the feasibility study and plans that led to this repository. D14 was settled with the `ferroan-db` crate.
