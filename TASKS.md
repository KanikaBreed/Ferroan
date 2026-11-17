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
| P0.1 | IN PROGRESS | | C# oracle in `tools/reference-oracle`: deterministic fixtures from libopenmetaverse, and verification of Rust-encoded bytes. Branch: `feat/p0-1-oracle`. `cargo xtask oracle verify` is still a manual step; a Linux CI job for it is not added yet. |
| P0.2 | TODO | P0.1 | `ferroan-llsd` from Aurora's `aurora-llsd`; XML, binary and notation match the C# encoder both ways; limits return errors; fuzz target compiles. |
| P0.3 | TODO | P0.1 | `ferroan-types`: UUID, vectors, quaternions, colours, prim shapes, permissions, TextureEntry, extra parameters, matching C# encodings. |
| P0.4 | TODO | P0.2, P0.3 | `ferroan-protocol` from Aurora's `aurora-msg`; `docs/PROTOCOL.md` reconciles the two message templates; every message round-trips against C# fixtures; framing and zero-coding. |
| P0.5 | TODO | P0.4 for replay | Captures: local capture patch for Tranquillity, HTTP capture proxy, an Aurora session recorded and redacted, replay test, `docs/CAPTURES.md`. |
| P0.6 | TODO | P0.1 | `ferroan-config`: resolves Tranquillity's shipped INI files to the same key/value set as C#. |
| P0.7 | TODO | | `ferroan-db`: sqlx on Tranquillity's schemas, migration versions checked, tests on Tranquillity-created SQLite databases. |
| P0.8 | TODO | P0.2 | `ferroan-http`: axum server, Robust XML replies and forms, LLSD over HTTP, reqwest client, matching `ServerUtils`. |
| P0.9 | TODO | P0.6, P0.7, P0.8 | Asset service pilot in `ferroan-robust`, behind a Tranquillity region; Aurora shows and uploads textures; porting rate measured and the plan re-estimated. |

Phases 1 to 7 get their rows when Phase 0 nears its gate. Their scope is in the [plan](docs/PLAN.md).
