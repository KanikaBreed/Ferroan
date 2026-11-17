# Action plan: Phase 0

This is the work for Phase 0 (Foundations), broken into packages a developer can pick up one at a time. Read [AGENTS.md](../AGENTS.md), [PORTING.md](PORTING.md) and [REFERENCES.md](REFERENCES.md) first. The work that runs in Tranquillity alongside this one is in [SLUA_TRANQUILLITY.md](SLUA_TRANQUILLITY.md).

**Phase 0 gate:** the protocol crate decodes and re-encodes every packet captured from a C# region session byte for byte, LLSD round-trips in all three encodings against the C# encoder, and a C# region runs against Ferroan's asset service, all checked by the live harness (P0.10); then the maintainer's Firestorm check (G0).

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
              │   P0.3b te    │
              │   P0.3c shape │
P0.6 config ──┤   P0.7 db ────┤
              └─> P0.8 http ──┴─> P0.9 asset service pilot
                                        │
             P0.4, P0.5, P0.9 ──────────┴─> P0.10 live harness (closes P0.5 and P0.9)
```

P0.1, P0.6 and P0.7 can start at once. P0.5's recording half can start at once too; its replay half needs P0.4.

**Automation first ([D15](DECISIONS.md)):** a check that a program can make is made by a program. A person with Firestorm checks each phase gate, not each package.

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

**Goal:** the shared scalar types every later crate uses. TextureEntry, extra parameters, prim shapes and permission masks are P0.3b and P0.3c, because each needs its own oracle fixtures and is a separate port.

- **Sources:** `libopenmetaverse/OpenMetaverse.Types` (UUID, Vector2, Vector3, Vector3d, Vector4, Quaternion, Color4, `Utils.FloatToUInt16`, `FloatToUnitUInt16`, `UInt16ToFloat`, `FloatZeroOneToByte`).
- **Steps:**
  1. Decide on `glam` for vectors and quaternions, as Aurora's crates use it, or plain structs. Record the decision in ARCHITECTURE.md.
  2. Port each type with its packet encoding: UUID byte order, the 12-byte packed quaternion (`GetBytes` and `ToBytes`), quaternion shorts, the 16-bit quantized values (`FloatToUInt16`), and colour bytes.
- **Acceptance:** every existing `family = bytes` fixture matches both ways, and the placeholder constant stays.

## P0.3b TextureEntry and extra parameters

**Goal:** the variable blobs object updates carry, in `ferroan-types`.

- **Sources:** `libopenmetaverse/OpenMetaverse/Primitives/TextureEntry.cs` and `Primitive.cs` extra parameters. Reuse Aurora's `aurora-prim` `te.rs`, `extra.rs` and `pack.rs` where they match OpenMetaverse.
- **Steps:** extend the oracle with TextureEntry and extra-parameter cases, then decode and re-encode those bytes.
- **Acceptance:** the new fixtures match both ways. Until this lands, `ferroan-protocol` may carry the blobs as bytes.

## P0.3c Prim shapes and permission masks

**Goal:** prim shape parameters and the permission bit masks, in `ferroan-types`.

- **Sources:** `libopenmetaverse/OpenMetaverse/Primitives` and Tranquillity's `Source/OpenSim.Framework/PrimitiveBaseShape.cs`, `PermissionMask` in `Source/OpenSim.Framework/Util.cs` and OpenMetaverse's `PermissionMask`.
- **Steps:** extend the oracle, then match those encodings.
- **Acceptance:** the new fixtures match both ways.

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
  4. Drive a session with `ferroan-bot` (P0.10): log in, walk, chat, rez and edit a prim, teleport within the region, log out. Record the bot revision and scenario. A Firestorm session can be added at the phase gate.
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
  - The region starts. `ferroan-bot` (P0.10) uploads a texture and fetches it and a library texture back unchanged. The recorded traffic matches what the C# asset service returns for the same requests. Firestorm showing textures is checked at the phase gate.
  - Record lines of C# ported, calendar days and hours spent, and bugs the fixtures caught.
  - Update [PLAN.md](PLAN.md) with the measured rate.

## P0.10 Live harness

**Goal:** check region and service behavior with no person at a viewer. A headless test client drives a local Tranquillity region, records the session, and compares results. It closes the open validation on P0.5 and P0.9, and every later phase uses it for every PR that changes behavior.

- **Sources:** Aurora's `crates/aurora-net` for the viewer side: `login.rs` (XML-RPC login), `circuit.rs` (acks and resends), `caps.rs` (seed and capabilities), `agent_update.rs`, `build.rs` and `fetch.rs`. libopenmetaverse's `GridClient` and `Programs`-style test clients show what a headless client needs. Tranquillity's standalone configuration and console commands (`create user`) for the region.
- **Do not depend on `aurora-net` as a crate.** It pulls in `aurora-assets`, which builds OpenJPEG, Vorbis and meshopt from C and C++. Adapt the files you need into Ferroan, with headers per [PORTING.md](PORTING.md), and build on `ferroan-protocol`, `ferroan-llsd` and `ferroan-http`. The bot then becomes the test client for Ferroan's own regions in Phase 2.
- **Steps:**
  1. `ferroan-bot` (`tools/bot`, Rust): log in over XML-RPC, open the circuit, handle acks and resends, fetch the seed capability, and run scenarios. Several bots can share a run.
  2. `cargo xtask live build`: clone or update Tranquillity at the pinned revision into `.scratch/live/` (never the reference checkout), apply `tools/capture/tranquillity-capture.patch`, and build it.
  3. `cargo xtask live up` and `down`: write a standalone SQLite configuration on free localhost ports, create test users without a person at the console, start the region, wait until it accepts logins, and stop it cleanly. `--assets ferroan` points the region's asset service at `ferroan-robust` through `ferroan-capture http-proxy`; `--assets csharp` uses the C# service through the same proxy. Run state stays under `.scratch/live/`.
  4. `cargo xtask live run <scenario>|all`: start the region, run scenarios, stop it, and write a report. Scenarios, each with checks made from server replies:
     - **login:** login succeeds, the region handshake completes, logout is clean.
     - **walk:** the avatar's reported position moves the way the sent controls ask.
     - **chat:** a second bot receives the first bot's local chat, with the right name and channel.
     - **build:** a prim is rezzed, renamed, scaled and moved; the object updates both bots see agree; it is deleted.
     - **teleport:** a teleport within the region lands at the target.
     - **texture:** a texture shipped with Tranquillity (`Source/OpenSim.Services.InventoryService/AppData/assets/TexturesAssetSet/`) is uploaded, then fetched back through the texture capability byte for byte. A library texture is fetched too. Both start with a valid JPEG 2000 codestream header.
     - **inventory:** the inventory skeleton and a folder fetch return the uploaded item.
  5. Captures: every run records UDP and HTTP. `ferroan-capture check`, then `redact`, then `replay`. The bots' text is synthetic, so the redacted session can be committed under `fixtures/captures/` with no person reading it first.
  6. Asset A/B: run the texture and inventory scenarios with `--assets csharp` and `--assets ferroan`, normalize the recorded asset traffic (dates, generated IDs), and require the same requests and replies. Write the differences to the report.
  7. CI: add a Linux job that installs .NET SDK 10 (pin the action by SHA). Tranquillity restores some packages from the OpenSim-NGC GitHub Packages feed (its `Docs/BUILDING.md`), so the job reads `GITHUB_ACTOR` and `GITHUB_TOKEN` for `nuget.config` from repository secrets the maintainer adds (`TRANQUILLITY_NUGET_USER`, `TRANQUILLITY_NUGET_TOKEN`, a token with `read:packages` only). If the secrets are absent, the job fails with a message naming them; it does not skip. The job builds Tranquillity at the pinned revision, and runs `cargo xtask live run all`. It also runs every `cargo xtask <oracle> verify`, which closes the "CI does not run the oracle" notes. It runs nightly, on `workflow_dispatch`, and on PRs that touch `tools/`, `xtask/`, `fixtures/` or a crate the scenarios use. Cache the Tranquillity build by revision.
  8. Turn the manual recipe in `docs/LIVE.md` into the `live` commands, keep the file as the harness guide (how to run it, what each scenario checks, how to add one), and keep `docs/MANUAL_CHECKS.md` (the maintainer's Firestorm checklist) in step with what the harness covers.
  9. Audit fixes from 17 November 2026: give `ferroan-robust` a listen-address setting (default unchanged, matching Tranquillity) and bind every harness process to `127.0.0.1`. Add a DECISIONS.md entry allowing `ring`, which `rustls` uses for TLS in `reqwest` and `sqlx`, as the crypto provider; no other native crate comes with it.
- **Acceptance:**
  - `cargo xtask live run all` passes on Linux locally and in CI, from a clean `.scratch/`.
  - A redacted bot session is committed and `replay` passes on it. P0.5 moves to `DONE`, recording that the session came from `ferroan-bot`.
  - The asset A/B run shows no differences, or each difference is explained in `docs/PROTOCOL.md` or filed as a task. P0.9 moves to `DONE`.
  - A failing check fails the job; nothing is skipped silently. A scenario that cannot run says why in the report.

## After Phase 0

### First login to a Ferroan region (P2.0)

Right after P0.10, before Phase 1 is finished, build the smallest Ferroan region a viewer can enter. It runs on the C# grid services the maintainer already runs (D2), so it does not wait for Phase 1.

- **Scope:** the LLUDP circuit and the region handshake, the seed capability and the event queue, terrain from the region's saved heightmap, the avatar standing at terrain height (no physics, Phase 3), appearance from the C# services, local chat, logout, and a teleport to and from a C# region next door.
- **Acceptance:** the live harness's login, chat and teleport scenarios pass against the Ferroan region, and their reports match the same scenarios against the C# region. Then the maintainer logs in with Firestorm and follows the first-login section that this package adds to `docs/MANUAL_CHECKS.md`.
- Split it into rows in TASKS.md before starting; it is more than two weeks of work.

When the gate passes, write Phase 1's packages the same way, from the scope in [PLAN.md](PLAN.md), and add their rows to TASKS.md. Every package that changes behavior a viewer or region can see adds or extends a `ferroan-bot` scenario. Once Ferroan has its own region (Phase 2), each scenario runs against both the C# region and the Ferroan region, and the two reports must agree.
