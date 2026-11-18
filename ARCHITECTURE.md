# Architecture

Ferroan is a Cargo workspace. Crates move from Planned to Implemented when their code lands.

## Implemented

| Crate | What it holds |
| --- | --- |
| `ferroan-types` | UUID, vectors, quaternions, colours, texture entries, extra parameters, prim shape fields and permission masks in OpenMetaverse and Tranquillity packet order. The Tranquillity revision constant stays |
| `ferroan-db` | sqlx pools for SQLite, MySQL and PostgreSQL. Checks Tranquillity's `migrations` versions and refuses to start when they differ. Does not migrate |
| `ferroan-config` | Tranquillity INI loading: Nini's parser, includes, and `${Section|key}` substitution. The region and grid loaders keep their different include order |
| `ferroan-llsd` | LLSD in XML, binary and notation, matching OpenMetaverse. Maps keep insertion order. Dates are tick counts, because one `f64` cannot hold both the XML text and the binary `AddSeconds` result |
| `ferroan-http` | Robust form bodies, `<ServerResponse>` XML and LLSD over HTTP, on axum and reqwest |
| `ferroan-protocol` | LLUDP messages generated from libopenmetaverse's `message_template.msg`, plus framing, zero-coding, appended acks and capture redaction. See [docs/PROTOCOL.md](docs/PROTOCOL.md) |
| `ferroan-capture` | The capture file, the HTTP logging proxy, and redaction. UDP recording is the local Tranquillity patch in [docs/CAPTURES.md](docs/CAPTURES.md) |
| `ferroan-bot` | Headless viewer in `tools/bot`: XML-RPC login, one LLUDP circuit, the seed capability, and the live-harness scenarios (login, walk, chat, build, teleport, texture, inventory). When the harness supplies a neighbour, `all` also teleports there and back. That crossing reads the source event queue. The first hop opens the destination with `UseCircuitCode`, and `CompleteAgentMovement` waits for `TeleportFinish`. The return sends `CompleteAgentMovement` on the circuit the source region kept. `cargo xtask live run all` runs against one Tranquillity region. `cargo xtask live run all grid` runs against GridServer and two regions |
| `ferroan-client` | One LLUDP circuit on tokio: `UseCircuitCode`, `RegionHandshake`, the viewer's reply, movement, throttle and window, pings, and `LogoutReply`. The seed answers a POST of capability names with `EventQueueGet`. The event queue holds an empty poll for 58 seconds and answers 502; delivered events use a randomly initialized id and `application/xml`. Throttles are still planned |
| `ferroan-services` | The asset service: the Robust asset routes and a SQLite store. Inventory and the other services are still planned |
| `ferroan-robust` | The grid-services process. This build serves the asset service on `[Network] port`. `[Network] bind_ip_address` chooses the listen address; empty means `0.0.0.0`, the same default as Tranquillity with that key unset |
| `ferroan-region` | One 256 metre region registered with C# Robust. Reads the region INI files through `ferroan-config`, registers/deregisters over the grid HTTP service, and accepts local `QueryAccess`/`CreateAgent` calls. Robust handles viewer login. One accepted agent uses `ferroan-client` on the configured UDP/HTTP listeners; logout releases the circuit and seed. Terrain, appearance and simulation remain planned. See [the crate guide](crates/ferroan-region/README.md) |
| `xtask` | Repository tasks: `cargo xtask check`, `oracle`, `schema`, `config`, `http`, `asset` and `live`. `live run` starts one region. `live run … grid` starts GridServer and two neighbouring regions. `live assets` compares C# and Ferroan asset HTTP. `live up --assets ferroan` leaves a C# region and `ferroan-robust` running on a seeded asset store; `live run assets --running` checks that setup and `live down` stops both |

The harness's grid controller records process identities for interrupted-run cleanup by `live down`. Grid and asset capture proxies bound replies at 70 seconds to cover C#'s 58-second event polls and explicitly close their single-exchange connections. Bot return teleports require fresh region evidence; diagnostic log tails redact secrets before truncation. Shared harness construction, example copying, redaction and run-root helpers keep standalone and grid behavior aligned.

The bot and region circuit share bounded sequence, duplicate-history and retry-window accounting in `ferroan-protocol::reliability`; each keeps its own ACK timing and retry policy. The region's immediate `UseCircuitCode` ACK has sequence zero; established ACKs use ordinary sequence numbers. Its handshake announces configured region name/type/owner and typed region settings matching C#'s fresh-region defaults. A valid `CompleteAgentMovement` triggers reliable `AgentMovementComplete`, using the accepted login position and region identity. Bot readiness requires a fresh arrival on the current circuit alongside handshake and seed completion. Terrain adjustment remains planned. Datagram errors do not tear down the listener. Exhausted retries release their window entries, and logout sends C#'s reliable, zero-coded reply without waiting for an ACK.

## Planned

### Programs

| Program | Replaces | Phase |
| --- | --- | --- |
| `ferroan-robust` | The rest of Robust: inventory, accounts, presence, grid, login, hypergrid, money | 1 |
| `ferroan-region` | The rest of the region simulator beyond grid login and one circuit | 2 |
| `ferroan-tools` | OAR and IAR tools, migration helpers, admin utilities | 5 to 7 |

### Server crates

| Crate | Purpose | Phase |
| --- | --- | --- |
| `ferroan-services` | Inventory, accounts, presence, grid, login, hypergrid, money server. Assets are implemented | 1 |
| `ferroan-scene` | Objects, prims, avatars, terrain, the frame loop | 2 |
| `ferroan-modules` | Land, estates, groups, inventory transfer, map, dynamic textures | 5 |
| `ferroan-physics` | The only crate that calls Jolt (native C++) | 3 |
| `ferroan-scripting` | The only crate that calls Linden's SLua engine (native C++); the `ll`, `os` and `bot` functions | 4 |

## Where code comes from

Aurora Viewer's `aurora-llsd` and `aurora-msg` become `ferroan-llsd` and `ferroan-protocol`, and `aurora-prim` supplies later prim volumes ([D11](docs/DECISIONS.md)). The message template Ferroan generates from is libopenmetaverse's, the one Tranquillity speaks. Texture entries, extra parameters and shape quantizers follow libopenmetaverse: Aurora's viewer packer does not emit the same bytes. Permission folding is ported from Tranquillity. Packet quaternions use OpenMetaverse `Quaternion.ToBytes`. Other C# code Aurora lacks is ported from libopenmetaverse and Tranquillity ([D12](docs/DECISIONS.md)). Revisions are pinned in [REFERENCES.md](docs/REFERENCES.md). The template differences are listed in [docs/PROTOCOL.md](docs/PROTOCOL.md).

The current compatibility target is Tranquillity `858321883f5487a5d192d36ea9d7b90fa468310c`. Its changes since `c85783e` leave the implemented Rust wire formats and migration catalog unchanged. The C# harness gains fixes for shared HTTP listener shutdown and unattended console defaults. The viewer asset capability's texture-as-material rejection, script fixes, and addin manifest corrections do not require changes to the implemented Rust services. Pin2 in [TASKS.md](TASKS.md) records the source review and validation.

## Vectors and quaternions

`ferroan-types` stores vectors and quaternions as `glam` 0.34 values (`Vec2`, `Vec3`, `DVec3`, `Vec4`, `Quat`), the same major version Aurora's `aurora-prim` uses. A `glam` value is not copied onto the wire: each field is written little-endian, which is what OpenMetaverse's `GetBytes` does. `Color4` is a plain struct. Its packet form is four quantized bytes, not a vector.

## Rules that shape the code

- **Wire compatibility.** Every crate that talks to the outside speaks exactly what the C# server speaks, so Rust and C# pieces can share a grid ([D2](docs/DECISIONS.md)).
- **Unsafe code in two places.** The workspace forbids unsafe code; only `ferroan-physics` and `ferroan-scripting` will be allowed it, each wrapping one native library.
- **Threads.** tokio runs networking and services. The region frame loop, Jolt and the SLua engine each get their own thread ([D6](docs/DECISIONS.md)). Linden's engine requires that each VM environment be used by one thread at a time.
- **No plug-in loading.** Region modules are compiled in. Third-party C# modules cannot load into Ferroan.
