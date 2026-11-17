# Tasks

Status values: `TODO`, `IN PROGRESS`, `AWAITING VALIDATION`, `DONE`, `BLOCKED`. Add an owner and branch when work starts, and evidence when it ends. Each Phase 0 row is a package in the [action plan](docs/ACTION_PLAN.md), which has its sources, steps and acceptance.

## Bootstrap

| ID | Status | Task and acceptance |
| --- | --- | --- |
| B01 | DONE | Private repository, licences and notices, contributor guides, plan and decisions, a Rust workspace with one placeholder crate, the check script and CI. No server behavior. |
| B02 | DONE | CI passed on Linux, Windows and macOS for the bootstrap commit `24746aa` ([run 44043347533](https://github.com/KanikaBreed/ferroan/actions/runs/44043347533)). |
| B03 | DONE | Licence changed to GPL-3.0-or-later (D4) so Aurora Viewer's crates can be reused (D11); reference checkouts pinned in docs/REFERENCES.md; Phase 0 action plan written. |
| B04 | TODO | Contributor onboarding for volunteers: label starter tasks, and decide how task claims are made (issue, PR or this file). |

## Prerequisite in Tranquillity (C#)

Tracked in the Tranquillity fork; packages are in [SLUA_TRANQUILLITY.md](docs/SLUA_TRANQUILLITY.md).

| ID | Status | Task and acceptance |
| --- | --- | --- |
| S0.1 | TODO | Build Linden's engine as a shared library for Linux and Windows in CI; its harness runs a sample LSL and SLua script. |
| S0.2 | TODO | C interface over Executor, reviewed header first. |
| S0.3 | TODO | .NET wrapper; an LSL script calls `llOwnerSay` from C# on Linux and Windows; empty host call timed. |
| S1.1 | TODO | Bindings generated from `ISystemAPI`; all 831 functions callable from LSL and SLua. |
| S1.2 | TODO | Compiler patch and `ll`, `ossl`, `bot` tables, with `OsslGate`. |
| S1.3 | TODO | Delays and deferred calls as yields; Phlox's syscall and sleep tests pass. |

## Phase 0: Foundations

Gate: the protocol crate decodes and re-encodes every packet captured from a C# region session byte for byte, LLSD round-trips in all three encodings against the C# encoder, and a C# region runs against Ferroan's asset service.

| ID | Status | Depends on | Task and acceptance |
| --- | --- | --- | --- |
| P0.1 | DONE | | C# oracle in `tools/reference-oracle`: 152 committed fixtures, two generates identical, `cargo xtask oracle verify` decoded them with libopenmetaverse `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`. PR #1, CI https://github.com/KanikaBreed/ferroan/actions/runs/44049951553 (Linux, Windows, macOS). `cargo xtask oracle verify` stays a manual step; a Linux CI job for it is not added yet. |
| P0.2 | DONE | P0.1 | `ferroan-llsd` adapted from Aurora Viewer `crates/aurora-llsd` at `48cb76e830a13965ac080128bb874afedda86d69`. XML, binary and notation match libopenmetaverse `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b`: `tests/oracle_fixtures.rs` decodes every LLSD fixture and re-encodes the round-trip cases to those bytes. `cargo xtask oracle verify` reported 152 fixtures match on Linux. PR [#4](https://github.com/KanikaBreed/ferroan/pull/4), CI [run 44067166631](https://github.com/KanikaBreed/ferroan/actions/runs/44067166631) (Linux, Windows, macOS) on `f9d8867`. CI does not run the oracle. A Windows `oracle verify` was not run; infinity text is the Linux ICU form. No viewer was shown an LLSD body. Depth and size limits return errors. `llsd-fuzz` compiles. |
| P0.3 | IN PROGRESS | P0.1 | UUID, vectors, quaternions, colours and `FloatToUInt16` in `ferroan-types`, matching the byte fixtures. `glam` for vectors and quaternions. Branch `feat/p0-3-types`. |
| P0.3b | TODO | P0.3 | TextureEntry and extra parameters in `ferroan-types`, from Aurora's `aurora-prim` where it matches, with new oracle fixtures. Until this lands, protocol messages carry those blobs as bytes. |
| P0.3c | TODO | P0.3 | Prim shape parameters and permission masks in `ferroan-types`, matching OpenMetaverse and Tranquillity, with new oracle fixtures. |
| P0.4 | TODO | P0.2, P0.3 | `ferroan-protocol` from Aurora's `aurora-msg`; `docs/PROTOCOL.md` reconciles the two message templates; every message round-trips against C# fixtures; framing and zero-coding. |
| P0.5 | TODO | P0.4 for replay | Captures: local capture patch for Tranquillity, HTTP capture proxy, an Aurora session recorded and redacted, replay test, `docs/CAPTURES.md`. |
| P0.6 | DONE | | `ferroan-config`: resolves Tranquillity's shipped region and grid INI files to the same key/value set as Nini 0.9.2.16 and the loaders at `713857f10b8e91a639a967d4354a4682b080793c`. Twenty-four cases; `cargo xtask config verify` matched locally, including the shipped dumps. PR [#3](https://github.com/KanikaBreed/ferroan/pull/3), CI [run 44061549807](https://github.com/KanikaBreed/ferroan/actions/runs/44061549807) (Linux, Windows, macOS). CI skips the two shipped cases because it has no reference checkout. Remote includes and `OpenSim.xml` are refused. |
| P0.7 | DONE | | `ferroan-db`: sqlx pools for SQLite, MySQL and PostgreSQL refuse to start when a store's `migrations` row is not the version Tranquillity reaches from empty. Fourteen SQLite databases were generated by Tranquillity `713857f10b8e91a639a967d4354a4682b080793c` and checked with `cargo xtask schema verify`. PR [#2](https://github.com/KanikaBreed/ferroan/pull/2), CI [run 44053450314](https://github.com/KanikaBreed/ferroan/actions/runs/44053450314) (Linux, Windows, macOS). MySQL and PostgreSQL were not run (`FERROAN_MYSQL_URL` and `FERROAN_POSTGRES_URL` unset). A Linux CI job with service containers is not added yet. `UseUTF16Encoding` with non-ASCII text was not tested. |
| P0.8 | TODO | P0.2 | `ferroan-http`: axum server, Robust XML replies and forms, LLSD over HTTP, reqwest client, matching `ServerUtils`. |
| P0.9 | TODO | P0.6, P0.7, P0.8 | Asset service pilot in `ferroan-robust`, behind a Tranquillity region; Aurora shows and uploads textures; porting rate measured and the plan re-estimated. |

Phases 1 to 7 get their rows when Phase 0 nears its gate. Their scope is in the [plan](docs/PLAN.md).
