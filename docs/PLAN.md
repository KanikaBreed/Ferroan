# Plan

Ferroan rebuilds Tranquillity in Rust, phase by phase, on grids where the rest is still C#. Each phase ends at a gate: a check that has to pass before the next phase starts. Phases 3 and 4 run side by side after Phase 2, as do Phases 5 and 6.

Effort is in person-months, estimated at 3,000 to 6,000 lines of C# ported per person-month. The asset pilot below is the first measurement of that rate. The estimate stands: one service session is too small a sample to resize the phases. The total is about 97 to 188 person-months (8 to 16 person-years). Because Ferroan is built by volunteers ([D5](DECISIONS.md)), calendar time depends on hours given: at 2.5 full-time equivalents, for example ten people at a quarter time each, it is about 3 to 6 years.

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

## Asset pilot measurement (17 November 2026)

P0.9 followed the asset HTTP routes, the service get/store/delete/exists behaviour, and the SQLite queries behind them, at Tranquillity `713857f10b8e91a639a967d4354a4682b080793c`. Those eight files are 1,402 lines:

| File | Lines |
| --- | --- |
| `Source/OpenSim.Server.Handlers/Asset/AssetServerConnector.cs` | 217 |
| `Source/OpenSim.Server.Handlers/Asset/AssetServerDeleteHandler.cs` | 121 |
| `Source/OpenSim.Server.Handlers/Asset/AssetServerGetHandler.cs` | 163 |
| `Source/OpenSim.Server.Handlers/Asset/AssetServerPostHandler.cs` | 114 |
| `Source/OpenSim.Server.Handlers/Asset/AssetsExistHandler.cs` | 79 |
| `Source/OpenSim.Services.AssetService/AssetService.cs` | 210 |
| `Source/OpenSim.Services.AssetService/AssetServiceBase.cs` | 99 |
| `Source/OpenSim.Data.SQLite/SQLiteAssetData.cs` | 399 |

`FetchAssetMetadataSet` and the MySQL and PostgreSQL stores landed in #59. The filesystem loader and the `show asset`, `dump asset` and `delete asset` functions landed in #60. The loader callback calls plugin `StoreAsset` when the id is new or the SHA1 of the bytes differs; HTTP `POST /assets/{id}` still returns false. `Migration.Update` is still unimplemented. The remote console is not wired to those three functions. TB1 (#61) matches the pin: a stored 8 is `Normal` and a stored 9 (Maptile mixed with bit 8) is `Maptile`, and `GET /assets/{id}` returns the C# documents.

Left out of the pilot: `MySQLAssetData.cs` (359 lines), `XAssetService.cs` (232) and `XAssetServiceBase.cs` (117). `AssetBase.cs` (380) is the type `XmlSerializer` writes. The Rust encoder matches those bytes.

The worktree was created at 14:05 UTC and the fixture tests passed at 14:40 UTC the same day. That is about 35 minutes of wall-clock time for one session, and it is the clock on the machine, not a timed person-hour study. One calendar day, one service. Scaling those 1,402 lines up to a person-month would move the 3,000 to 6,000 band, and this sample does not justify that. The phase table stays as estimated.

Bugs the C# fixtures caught while matching `XmlSerializer`:

- Flag value 8 is not a valid `AssetFlags` value, and serialization throws. The named bits are 1 (`Maptile`), 2 (`Rewritable`) and 4 (`Collectable`). Value 7 is the text `Maptile Rewritable Collectable`.
- Empty strings and empty byte arrays are self-closing elements with a space before the slash (`<Data />`).
- CR, LF and tab stay raw characters in element text.
- An empty array is one self-closing namespaced element, with no trailing newline.

The region and Aurora checks for this pilot are still open. See [TASKS.md](../TASKS.md).

## What is Rust, native or dropped

| Part | Decision |
| --- | --- |
| Region core, grid services, client protocol, databases, script host and functions, meshing, JPEG 2000, drawing, map tiles, add-ons, configuration | Rust |
| Script VM and compilers | Native: Linden Lab's SLua engine (C++), one engine for LSL, SLua, OSSL (an `ossl` table) and InWorldz bot functions (a `bot` table) |
| Physics engine | Native: Jolt 5.6 (C++) through `oxijolt-sys` |
| BulletSim, ubODE, POS and basic physics | Dropped; Jolt only |
| YEngine and its language extensions | Dropped; YEngine and XEngine state from hypergrid visitors is still imported |
| Gloebit and XmlRpcGroups | Dropped until someone needs them |
| Third-party C# region modules | Cannot load into Rust |
| Saved script state from Phlox and YEngine | Carried over through the portable script state format ([SLUA_TRANQUILLITY.md](SLUA_TRANQUILLITY.md)), written by the C# server before the cut-over; a script whose state does not convert stays on a C# region until it does |

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
- **Phase 1:** assets (all three of Tranquillity's backends: `AssetService` on SQLite, MySQL and PostgreSQL, `XAssetService` and FSAssets on MySQL and PostgreSQL), inventory, accounts, authentication, grid user, presence, grid, avatars, friends, offline IM, Groups V2, search, login over XML-RPC, hypergrid (gatekeeper, user agent, HG inventory and assets), and the money server. Each runs behind a C# grid as soon as it passes its fixtures.
- **Asset system:** a new asset backend behind the same routes ([ASSET_SYSTEM.md](ASSET_SYSTEM.md), D23 to D28, the `AS` rows in TASKS.md). Its core follows P1.A1 to P1.A3 in Phase 1; delivery to viewers comes with Phase 2's region, and Hypergrid content with P1.14.
- **Phase 2:** the LLUDP circuit, CAPS and the event queue, the scene, object and inventory updates, permissions, appearance and baking, teleports and crossings to and from C# regions, and loading OAR files with terrain-height standing only. After P2.0j, rows P2.0e2, P2.0e3, P2.1a to P2.12 and G3 in [TASKS.md](../TASKS.md) (queue Q40 to Q85) take the region to the gate under [D38 and D39](DECISIONS.md): a BasicPhysics walking stand-in until Phase 3, "next door" as teleports both ways plus child agents, viewer-side bakes, and all three region-store databases. What is left is mapped in [plans/PHASE2-GATE.md](plans/PHASE2-GATE.md).
- **Phase 3:** Jolt behind `ferroan-physics`, prim, sculpt and mesh shapes, the avatar controller, vehicles, buoyancy and collision events. The rows are in [plans/PHASE3-PHYSICS.md](plans/PHASE3-PHYSICS.md) and the Phase 3 section of [TASKS.md](../TASKS.md) (queue Q100 onward in [WORK_QUEUE.md](../WORK_QUEUE.md)).
  - The arrival landing ray cast is P3.16. At Pin8 `c401bceecd`, `ScenePresence.MakeRootAgent` (`Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs` 1543–1602) casts a ray down from above the arrival point with `RayFilterFlags.PrimsNonPhantomAgents` on a login, a teleport to a location or a hypergrid login, when the arrival is not carried, the landing point was not moved, and the engine supports filtered ray casts. It lands the newcomer on the first prim or avatar capsule it hits, so a second avatar arriving at the same point stands on the first (about 2 m up). Jolt (`JoltScene.cs` 2247), ubODE (`ODEScene.cs` 2370) and BulletSim (`BSScene.cs` 968) support it. BasicPhysics does not (`PhysicsScene.cs` 347–350), so the D38 walking stand-in correctly has no ray cast, and a `--physics basicphysics` C# region lands at ground height too. Port it with Jolt's ray cast, with a fixture of two avatars logging in at the same point on a C# region running Jolt. Until then, compare arrivals against a C# region running basicphysics, not the harness default ubODE.
- **Phase 4:** the SLua engine plan's C interface called from Rust, the 831 `ll`, `os` and `bot` functions, the scheduler, events, time slices, memory limits and state saving, and reading and writing portable script state, YEngine snapshots and the `<Variables>` form for hypergrid.
- **Phase 5:** land and parcels, estates, groups in the region, inventory transfer, the money module, NPCs and bots, linkset data, Experiences, map tiles, dynamic textures, OAR and IAR, media, sound, remote admin, monitoring.
- **Phase 6:** WebRTC voice through the external Janus server, search, offline IM, the mute list.
- **Phase 7:** the operator migration guide and tooling, beta grids, performance tuning, and retiring the C# processes.
