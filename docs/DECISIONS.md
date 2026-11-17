# Decisions

Settled decisions. To change one, open a PR that edits this file and says why.

| # | Decision | Chosen | Notes |
| --- | --- | --- | --- |
| D1 | Do the SLua engine plan in today's C# server first? | Yes. Run Linden's SLua engine in Tranquillity, and keep using C# until Ferroan is stable | Phase 4 reuses that work's C interface and conformance suite |
| D2 | Mixed grids or one big switch | Dual support until the switch: Rust and C# pieces run on the same grid | Every phase keeps wire compatibility with the C# server |
| D3 | Where the code lives | This repository, run by the team | Private for now |
| D4 | Licence | Keep each ported file's licence and its references | MPL-2.0 files stay MPL-2.0. New code is MPL-2.0, as in Tranquillity. Each ported file keeps its original notices and names its C# source ([PORTING.md](PORTING.md)) |
| D5 | Team | Volunteers | Calendar time depends on hours given ([PLAN.md](PLAN.md)) |
| D6 | Async runtime and HTTP stack | tokio for networking and services; axum and hyper for HTTP servers, reqwest as client. The region frame loop, Jolt and the SLua engine run on their own threads | tokio is the most widely used runtime, and axum, hyper, reqwest and sqlx build on it. A frame loop needs a fixed tick, and both native libraries expect a thread of their own |
| D7 | Feature freeze in C# | Freeze each part's features once its Rust phase starts | Bug fixes still go into both |
| D8 | Native libraries | Only Jolt and Linden Lab's SLua engine | A third needs its own decision here |
| D9 | Physics engines | Jolt only | BulletSim, ubODE, POS and basic physics are not ported |
| D10 | One script engine | Linden's SLua engine runs LSL, SLua, OSSL and bot functions | Phlox's VM and YEngine are not ported; YEngine's language extensions are unsupported |

D1 to D7 were settled on 17 November 2026. D8 to D10 come from the feasibility study and plans that led to this repository.
