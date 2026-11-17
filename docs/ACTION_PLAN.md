# Action plan: Phase 0

This is the work for Phase 0 (Foundations), broken into packages a developer can pick up one at a time. Read [AGENTS.md](../AGENTS.md), [PORTING.md](PORTING.md) and [REFERENCES.md](REFERENCES.md) first. The work that runs in Tranquillity alongside this one is in [SLUA_TRANQUILLITY.md](SLUA_TRANQUILLITY.md).

**Phase 0 gate:** the protocol crate decodes and re-encodes every packet captured from a C# region session byte for byte, LLSD round-trips in all three encodings against the C# encoder, and a C# region runs against Ferroan's asset service.

## How to work a package

1. Take the next package whose dependencies are done. Set its row in [TASKS.md](../TASKS.md) to `IN PROGRESS` with the branch name.
2. Create a task worktree from `origin/main` (see [AGENTS.md](../AGENTS.md) section 5).
3. Read the listed sources at their pinned revisions. Copy or port with the headers [PORTING.md](PORTING.md) requires.
4. Meet every acceptance point. Run `cargo xtask check`.
5. Open a PR using the template, with the source revisions, the checks run and anything still pending. Update TASKS.md with the evidence.
6. Stop at the PR. Merging is the maintainer's call.

If a package turns out bigger than about two weeks of work, split it in TASKS.md before going on.

## Order

```text
P0.1 oracle ──┬─> P0.2 llsd ──┐
              ├─> P0.3 types ─┼─> P0.4 protocol ──> P0.5 capture replay
              │               │
P0.6 config ──┤   P0.7 db ────┤
              └─> P0.8 http ──┴─> P0.9 asset service pilot
```

P0.1, P0.6 and P0.7 can start at once. P0.5's recording half can start at once too; its replay half needs P0.4.

## P0.1 C# oracle

**Goal:** a small .NET tool that produces and checks fixtures with the exact C# code Tranquillity uses, so Rust is tested against the real encoder, not against itself.

- **Sources:** `libopenmetaverse` (OpenMetaverse, OpenMetaverse.StructuredData, OpenMetaverse.Types), `OpenSim-Tranquillity` (for Robust's XML helpers in `Source/OpenSim.Server.Base/ServerUtils.cs` later). Pattern to follow: `~/Git/dcs-srs-rust/tools/reference-fixtures` (its Python driver becomes an `xtask` here, per [D13](DECISIONS.md)).
- **Output:** `tools/reference-oracle/` (.NET 10 console project referencing the reference checkout's projects by path, never copying them), `cargo xtask oracle build|generate|verify` to build and run it, fixtures under `fixtures/oracle/`. The oracle is the one C# program in this repository, allowed by D13 because it must run Tranquillity's own C# code.
- **Modes:** `generate` writes fixtures (a JSON manifest plus binary payloads); `verify` reads bytes Rust produced and decodes them with C#, reporting any field that differs.
- **Acceptance:**
  - The tool builds against the pinned `libopenmetaverse` with a path set by an environment variable (`FERROAN_REF`, default `~/Git/reference`).
  - Fixtures are deterministic: two runs produce identical files.
  - CI does not need .NET; the generated fixtures are committed, and `cargo xtask oracle verify` is a manual step until a Linux CI job is added (note it in TASKS.md).

## P0.2 `ferroan-llsd`

**Goal:** LLSD in XML, binary and notation, matching OpenMetaverse byte for byte.

- **Sources:** reuse Aurora's `crates/aurora-llsd` (D11); check against `libopenmetaverse/OpenMetaverse.StructuredData/LLSD/` (`XmlLLSD.cs`, `BinaryLLSD.cs`, `NotationLLSD.cs`).
- **Steps:**
  1. Copy `aurora-llsd` into `crates/ferroan-llsd`, with Aurora's notice and revision in each file.
  2. Add oracle fixtures from P0.1 for every type: undef, boolean, integer, real (including NaN, infinities and -0), string, UUID, date, URI, binary, map, array, nesting.
  3. Fix differences in favour of OpenMetaverse's output, and record each one in the crate's README.
- **Acceptance:** Rust decodes every C# fixture to the same value, and Rust's encoding passes `oracle verify`, in all three formats. Input limits (nesting depth, total size) return errors, never panic. A fuzz target compiles.

## P0.3 `ferroan-types`

**Goal:** the shared types every later crate uses.

- **Sources:** `libopenmetaverse/OpenMetaverse.Types` (UUID, Vector3, Vector3d, Quaternion, Color4, Utils byte helpers), `libopenmetaverse/OpenMetaverse/Primitives` and Tranquillity's `Source/OpenSim.Framework/PrimitiveBaseShape.cs` (prim shape parameters), `PermissionMask` in `Source/OpenSim.Framework/Util.cs` and OpenMetaverse's permission flags. Reuse Aurora's `aurora-prim` `te.rs`, `extra.rs` and `pack.rs` for TextureEntry and extra parameters where they fit.
- **Steps:**
  1. Decide on `glam` for vectors and quaternions, as Aurora's crates use it, or plain structs. Record the decision in ARCHITECTURE.md.
  2. Port each type with its packet encoding: UUID byte order, the 12-byte packed quaternion, the 16-bit quantized values (`FloatToUInt16`), TextureEntry and extra parameters.
- **Acceptance:** every encoding matches oracle fixtures both ways, and the placeholder constant stays.

## P0.4 `ferroan-protocol`

**Goal:** every LLUDP message, encoded and decoded, with packet framing.

- **Sources:** reuse Aurora's `crates/aurora-msg` (`build.rs` generates code from `message_template.msg`). Compare `Aurora-Viewer/crates/aurora-msg/message_template.msg` (the SL viewer's, 483 messages) with `libopenmetaverse/data/message_template.msg` (478 messages, what Tranquillity speaks). Framing behavior: `libopenmetaverse/OpenMetaverse/_Packets_.cs`, Tranquillity's `Source/OpenSim.Region.ClientStack.LindenUDP/` (`LLUDPServer.cs` and the packet handling around it). Aurora's `aurora-net` shows the viewer's side of acks and resends.
- **Steps:**
  1. Write `docs/PROTOCOL.md`: which template Ferroan generates from, and every message whose number, frequency, trust, zero-coding or blocks differ between the two templates, with the choice made for each. A server has to understand what Tranquillity understands.
  2. Copy `aurora-msg` into `crates/ferroan-protocol`. Make sure the generator emits both directions for every message, since a server decodes what viewers send and encodes what they receive.
  3. Implement the framing: flags, sequence numbers, extra header, appended acks, zero-coding, and the size limit.
- **Acceptance:** for every message in libopenmetaverse's template, C#-encoded fixtures (P0.1, with deterministic field values, zero-coded where the template says) decode and re-encode byte for byte, and Rust-encoded packets pass `oracle verify`. Malformed packets return errors. A fuzz target compiles.

## P0.5 Captures

**Goal:** real traffic from a Tranquillity session as fixtures. This is Phase 0's gate, and later phases depend on it.

- **Recording half (can start at once):**
  1. Build Tranquillity at the reference revision in a worktree or `.scratch/`, never in the reference checkout. Run it as a standalone grid with SQLite on localhost, with a test account.
  2. Record UDP: write a small patch for Tranquillity's LLUDP server that writes raw inbound and outbound datagrams (direction, timestamp, bytes) to a file when a debug option is set. Keep it as `tools/capture/tranquillity-capture.patch`, applied only to the local build and never sent upstream.
  3. Record HTTP: write `tools/capture`, a Rust crate (`ferroan-capture`) whose `http-proxy` subcommand is a small logging reverse proxy. Point the region's service URLs at it to record region-to-service traffic, and put it in front of CAPS to record viewer traffic.
  4. Drive a session with Aurora: log in, walk, chat, rez and edit a prim, teleport within the region, log out. Record the viewer revision and steps.
  5. Add a `redact` subcommand to `ferroan-capture`: replace names, IP addresses, session and circuit IDs, passwords and tokens with fixed placeholders, consistently. Commit only redacted captures, under `fixtures/captures/`.
- **Replay half (after P0.4):** a test that decodes every recorded datagram and re-encodes it byte for byte, listing any message that fails.
- **Acceptance:** at least one full redacted session is committed. The replay test passes, or every failure is a filed task. The capture steps are written up in `docs/CAPTURES.md`.

## P0.6 `ferroan-config`

**Goal:** read Tranquillity's INI files exactly as Tranquillity does.

- **Sources:** Tranquillity's config loading (`Source/OpenSim.Server.RegionServer/ConfigurationLoader.cs` and the grid server's equivalent in `Source/OpenSim.Server.Base`), and the shipped `Source/OpenSim.Server.RegionServer/AppData/RegionServerDefaults.ini` and `RegionServer.ini.example`, and `Source/OpenSim.Server.GridServer/AppData/GridServer.ini.example` (Tranquillity's Robust).
- **Behavior to match:** sections and keys, comments, quoting, `[Include-*]` sections, `${Section|key}` substitution, environment overrides if Tranquillity has them, and what wins when a key is set twice.
- **Acceptance:** the oracle (P0.1, extended) dumps the resolved key/value set for the shipped files, and Rust produces the same set. Unknown keys are reported, not fatal.

## P0.7 `ferroan-db`

**Goal:** database access on Tranquillity's schemas, unchanged.

- **Sources:** `Source/OpenSim.Data.SQLite`, `Source/OpenSim.Data.MySQL`, `Source/OpenSim.Data.PGSQL` and their `Resources/*.migrations`; the `migrations` table each store keeps.
- **Steps:** sqlx pools for all three backends; read each store's migration version and refuse to start if it differs from the version this build expects; no migrations of Ferroan's own.
- **Acceptance:** tests on SQLite databases created by Tranquillity at the reference revision. MySQL and PostgreSQL tests run where a server is available (a Linux CI job with service containers can come later; note it in TASKS.md).

## P0.8 `ferroan-http`

**Goal:** the HTTP pieces every service and CAPS handler needs.

- **Sources:** `Source/OpenSim.Server.Base/ServerUtils.cs` (Robust's XML reply format and form parsing), `Source/OpenSim.Framework.Servers.HttpServer`, `Source/OpenSim.Services.Connectors` (what clients send).
- **Steps:** an axum server with Robust-style routing, the XML reply builder and parser, LLSD request and response handling over `ferroan-llsd`, and a reqwest client.
- **Acceptance:** replies and requests match oracle fixtures from `ServerUtils`, and limits on body size and nesting return errors.

## P0.9 Asset service pilot

**Goal:** the first real Ferroan service, running behind a C# grid, and a measured porting rate.

- **Sources:** `Source/OpenSim.Server.Handlers/Asset/`, `Source/OpenSim.Services.AssetService/`, `Source/OpenSim.Data.SQLite/SQLiteAssetData.cs` (and `Source/OpenSim.Data.MySQL/MySQLAssetData.cs`), and the `[AssetService]` section of `GridServer.ini.example`.
- **Steps:**
  1. Create `ferroan-services` (asset service only) and the `ferroan-robust` program.
  2. Serve the asset routes the C# connectors call: get, data, metadata, store, delete and exists.
  3. Point a Tranquillity region at it, through the HTTP capture proxy to record the traffic.
- **Acceptance:**
  - The region starts. Aurora shows textures and can upload one. The recorded traffic matches what the C# asset service returns for the same requests.
  - Record lines of C# ported, calendar days and hours spent, and bugs the fixtures caught.
  - Update [PLAN.md](PLAN.md) with the measured rate.

## After Phase 0

When the gate passes, write Phase 1's packages the same way, from the scope in [PLAN.md](PLAN.md), and add their rows to TASKS.md.
