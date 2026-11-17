# Architecture

Ferroan is a Cargo workspace. Today it holds one placeholder crate. Everything else on this page is **planned**, and each entry moves to "Implemented" when its code lands.

## Implemented

| Crate | What it holds |
| --- | --- |
| `ferroan-types` | Placeholder. Records the Tranquillity reference revision. Shared types land here in Phase 0 |
| `xtask` | Repository tasks: `cargo xtask check` (Markdown links, formatting, clippy, tests) and `cargo xtask oracle` (the C# libopenmetaverse fixture tool) |

## Planned

### Programs

| Program | Replaces | Phase |
| --- | --- | --- |
| `ferroan-robust` | Robust, the grid services server | 1 |
| `ferroan-region` | OpenSim.exe, the region simulator | 2 |
| `ferroan-tools` | OAR and IAR tools, migration helpers, admin utilities | 5 to 7 |

### Server crates

| Crate | Purpose | Phase |
| --- | --- | --- |
| `ferroan-services` | Assets, inventory, accounts, presence, grid, login, hypergrid, money server | 1 |
| `ferroan-scene` | Objects, prims, avatars, terrain, the frame loop | 2 |
| `ferroan-client` | The LLUDP circuit, throttles, CAPS, the event queue | 2 |
| `ferroan-modules` | Land, estates, groups, inventory transfer, map, dynamic textures | 5 |
| `ferroan-physics` | The only crate that calls Jolt (native C++) | 3 |
| `ferroan-scripting` | The only crate that calls Linden's SLua engine (native C++); the `ll`, `os` and `bot` functions | 4 |

### Shared crates

| Crate | Purpose | Phase |
| --- | --- | --- |
| `ferroan-protocol` | Packets generated from the viewer's `message_template.msg` | 0 |
| `ferroan-llsd` | LLSD in XML, binary and notation | 0 |
| `ferroan-types` | UUIDs, vectors, rotations, prim shapes, permissions | 0 |
| `ferroan-db` | sqlx on Tranquillity's MySQL, PostgreSQL and SQLite schemas | 0 |
| `ferroan-config` | Reads Tranquillity's INI files | 0 |
| `ferroan-http` | HTTP server and client for Robust's forms and CAPS | 0 |

## Where code comes from

Aurora Viewer's `aurora-llsd` and `aurora-msg` become `ferroan-llsd` and `ferroan-protocol`, and `aurora-prim` supplies TextureEntry, extra parameters and later prim volumes ([D11](docs/DECISIONS.md)). C# code Aurora lacks is ported from libopenmetaverse and Tranquillity ([D12](docs/DECISIONS.md)). Revisions are pinned in [REFERENCES.md](docs/REFERENCES.md).

## Rules that shape the code

- **Wire compatibility.** Every crate that talks to the outside speaks exactly what the C# server speaks, so Rust and C# pieces can share a grid ([D2](docs/DECISIONS.md)).
- **Unsafe code in two places.** The workspace forbids unsafe code; only `ferroan-physics` and `ferroan-scripting` will be allowed it, each wrapping one native library.
- **Threads.** tokio runs networking and services. The region frame loop, Jolt and the SLua engine each get their own thread ([D6](docs/DECISIONS.md)). Linden's engine requires that each VM environment be used by one thread at a time.
- **No plug-in loading.** Region modules are compiled in. Third-party C# modules cannot load into Ferroan.
