# Architecture

Ferroan is a Cargo workspace. Crates move from Planned to Implemented when their code lands.

## Implemented

| Crate | What it holds |
| --- | --- |
| `ferroan-types` | UUID, vectors, quaternions and colours in OpenMetaverse's packet order. The Tranquillity revision constant stays |
| `ferroan-db` | sqlx pools for SQLite, MySQL and PostgreSQL. Checks Tranquillity's `migrations` versions and refuses to start when they differ. Does not migrate |
| `ferroan-config` | Tranquillity INI loading: Nini's parser, includes, and `${Section|key}` substitution. The region and grid loaders keep their different include order |
| `ferroan-llsd` | LLSD in XML, binary and notation, matching OpenMetaverse. Maps keep insertion order. Dates are tick counts, because one `f64` cannot hold both the XML text and the binary `AddSeconds` result |
| `ferroan-http` | Robust form bodies, `<ServerResponse>` XML and LLSD over HTTP, on axum and reqwest |
| `xtask` | Repository tasks: `cargo xtask check`, `oracle`, `schema`, `config` and `http` |

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
| `ferroan-types` | TextureEntry, extra parameters, prim shapes and permission masks, still to land in this crate | 0 |

## Where code comes from

Aurora Viewer's `aurora-llsd` and `aurora-msg` become `ferroan-llsd` and `ferroan-protocol`, and `aurora-prim` supplies TextureEntry, extra parameters and later prim volumes ([D11](docs/DECISIONS.md)). C# code Aurora lacks is ported from libopenmetaverse and Tranquillity ([D12](docs/DECISIONS.md)). Revisions are pinned in [REFERENCES.md](docs/REFERENCES.md).

## Vectors and quaternions

`ferroan-types` stores vectors and quaternions as `glam` 0.34 values (`Vec2`, `Vec3`, `DVec3`, `Vec4`, `Quat`), the same major version Aurora's `aurora-prim` uses. A `glam` value is not copied onto the wire: each field is written little-endian, which is what OpenMetaverse's `GetBytes` does. `Color4` is a plain struct. Its packet form is four quantized bytes, not a vector.

## Rules that shape the code

- **Wire compatibility.** Every crate that talks to the outside speaks exactly what the C# server speaks, so Rust and C# pieces can share a grid ([D2](docs/DECISIONS.md)).
- **Unsafe code in two places.** The workspace forbids unsafe code; only `ferroan-physics` and `ferroan-scripting` will be allowed it, each wrapping one native library.
- **Threads.** tokio runs networking and services. The region frame loop, Jolt and the SLua engine each get their own thread ([D6](docs/DECISIONS.md)). Linden's engine requires that each VM environment be used by one thread at a time.
- **No plug-in loading.** Region modules are compiled in. Third-party C# modules cannot load into Ferroan.
