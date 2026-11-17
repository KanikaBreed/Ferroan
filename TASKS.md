# Tasks

Status values: `TODO`, `IN PROGRESS`, `AWAITING VALIDATION`, `DONE`, `BLOCKED`. Add an owner and branch when work starts, and evidence when it ends. The phases and gates are in the [plan](docs/PLAN.md).

## Bootstrap

| ID | Status | Task and acceptance |
| --- | --- | --- |
| B01 | DONE | Private repository, licences and notices, contributor guides, plan and decisions, a Rust workspace with one placeholder crate, the check script and CI. No server behavior. |
| B02 | TODO | CI passes on Linux, Windows and macOS for the bootstrap commit. Record the run here. |
| B03 | TODO | Contributor onboarding for volunteers: label starter tasks, and decide how task claims are made (issue, PR or this file). |

## Prerequisite in Tranquillity (C#)

| ID | Status | Task and acceptance |
| --- | --- | --- |
| S01 | TODO | The SLua engine plan runs in Tranquillity first ([D1](docs/DECISIONS.md)). Its C interface and conformance suite are reused by Phase 4. Tracked in the Tranquillity repository, not here. |

## Phase 0: Foundations

Gate: the protocol crate decodes and re-encodes every packet captured from a C# region session, byte for byte, and LLSD round-trips in all three encodings.

| ID | Status | Task and acceptance |
| --- | --- | --- |
| F01 | TODO | `ferroan-types`: UUIDs, vectors, rotations, prim shapes, permission masks, with tests against values taken from C#. |
| F02 | TODO | `ferroan-llsd`: XML, binary and notation, checked against payloads captured from Tranquillity. Decide whether to build on serde-llsd. |
| F03 | TODO | `ferroan-protocol`: generate every packet from `message_template.msg` in a build script, with zero-coding, acks and resends. |
| F04 | TODO | Capture rig: record viewer-to-region, region-to-Robust and region-to-region traffic on a test grid, redacted, as fixtures. |
| F05 | TODO | `ferroan-config`: read Tranquillity's INI files and includes; report unknown keys. |
| F06 | TODO | `ferroan-db`: sqlx pools on Tranquillity's schemas without changing them. |
| F07 | TODO | `ferroan-http`: server and client for Robust's REST and XML forms and for CAPS. |
| F08 | TODO | Pilot: port the asset service, run it behind a test grid with C# regions, and measure lines ported per person-month. Re-estimate the plan from that rate. |

Phases 1 to 7 get their task rows when Phase 0 nears its gate. Their scope is in the [plan](docs/PLAN.md).
