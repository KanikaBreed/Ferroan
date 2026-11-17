# Plan

Ferroan rebuilds Tranquillity in Rust, phase by phase, on grids where the rest is still C#. Each phase ends at a gate: a check that has to pass before the next phase starts. Phases 3 and 4 run side by side after Phase 2, as do Phases 5 and 6.

Effort is in person-months, estimated at 3,000 to 6,000 lines of C# ported per person-month. The Phase 0 pilot (task F08) replaces that rate with a measured one. The total is about 97 to 188 person-months (8 to 16 person-years). Because Ferroan is built by volunteers ([D5](DECISIONS.md)), calendar time depends on hours given: at 2.5 full-time equivalents, for example ten people at a quarter time each, it is about 3 to 6 years.

| Phase | Person-months | Gate |
| --- | --- | --- |
| 0. Foundations | 6 to 12 | Every packet captured from a C# region session decodes and re-encodes byte for byte; LLSD round-trips in all three encodings |
| 1. Grid services | 16 to 32 | A live grid runs on Rust services with C# regions, and hypergrid visits work both ways |
| 2. Region simulator core | 25 to 49 | In Firestorm and Aurora, a user logs in to a Rust region, walks, builds, chats and teleports to and from C# regions next door |
| 3. Physics (beside 4) | 6 to 11 | Vehicle, walking and collision tests give the same results as the C# Jolt module |
| 4. Scripts (beside 3) | 7 to 13 | The SLua engine plan's conformance suite passes on a Rust region |
| 5. Region modules and content (beside 6) | 24 to 47 | OAR and IAR round trips match the C# server's, and the feature checklist reaches parity |
| 6. Add-ons (beside 5) | 7 to 14 | Voice, search and offline IM work in Rust regions |
| 7. Cut-over | 6 to 10 | Production grids run with no C# process |

## What is Rust, native or dropped

| Part | Decision |
| --- | --- |
| Region core, grid services, client protocol, databases, script host and functions, meshing, JPEG 2000, drawing, map tiles, add-ons, configuration | Rust |
| Script VM and compilers | Native: Linden Lab's SLua engine (C++), one engine for LSL, SLua, OSSL (an `ossl` table) and InWorldz bot functions (a `bot` table) |
| Physics engine | Native: Jolt 5.6 (C++) through `oxijolt-sys` |
| BulletSim, ubODE, POS and basic physics | Dropped; Jolt only |
| YEngine and its language extensions | Dropped |
| Gloebit and XmlRpcGroups | Dropped until someone needs them |
| Third-party C# region modules | Cannot load into Rust |
| Saved script state from Phlox and YEngine | Not carried over; scripts restart once |

## Rust building blocks

These crates were checked on 17 November 2026; versions move.

| Need | Crate |
| --- | --- |
| Async runtime, HTTP | tokio, axum, hyper, reqwest |
| Databases | sqlx 0.9 (MySQL, PostgreSQL, SQLite) |
| Physics | oxijolt-sys 1.5 (Jolt 5.6 through joltc) |
| Convex decomposition | parry3d's V-HACD |
| JPEG 2000 | j2k (pure Rust, encode and decode) |
| 2D drawing | tiny-skia, plus a font crate (tiny-skia has no text) |
| LLSD | serde-llsd, or our own crate |

## Phase scope

- **Phase 0:** see [TASKS.md](../TASKS.md).
- **Phase 1:** assets (including FS assets), inventory, accounts, authentication, grid user, presence, grid, avatars, friends, offline IM, Groups V2, search, login over XML-RPC, hypergrid (gatekeeper, user agent, HG inventory and assets), and the money server. Each runs behind a C# grid as soon as it passes its fixtures.
- **Phase 2:** the LLUDP circuit, CAPS and the event queue, the scene, object and inventory updates, permissions, appearance and baking, teleports and crossings to and from C# regions, and loading OAR files with terrain-height standing only.
- **Phase 3:** Jolt behind `ferroan-physics`, prim, sculpt and mesh shapes, the avatar controller, vehicles, buoyancy and collision events.
- **Phase 4:** the SLua engine plan's C interface called from Rust, the 831 `ll`, `os` and `bot` functions, the scheduler, events, time slices, memory limits and state saving.
- **Phase 5:** land and parcels, estates, groups in the region, inventory transfer, the money module, NPCs and bots, linkset data, Experiences, map tiles, dynamic textures, OAR and IAR, media, sound, remote admin, monitoring.
- **Phase 6:** WebRTC voice through the external Janus server, search, offline IM, the mute list.
- **Phase 7:** the operator migration guide and tooling, beta grids, performance tuning, and retiring the C# processes.
