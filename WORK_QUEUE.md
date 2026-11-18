# Work queue

The project manager orders Ferroan's work here. [TASKS.md](TASKS.md) records what each task is and the evidence that it is done. This file says what to do next and in what order.

## How the coder uses it

1. Take the first item marked `READY`. Do not start anything else: not other plan items, not Phase 1, not B04.
2. Work it the way [AGENTS.md](AGENTS.md) describes: a task worktree, the C# reference at the pinned revision, the checks, and a PR using the template.
3. Set the item to `IN PROGRESS` with the branch when you start, and to `DONE` with the PR number when it merges. Keep the matching TASKS.md rows in step.
4. If an item is wrong, too big or blocked, set it to `BLOCKED`, add the reason under the item, and stop. Do not change an item's scope yourself.
5. When no item is `READY`, stop and say so.

Only the manager adds, removes, reorders or rewrites items. The coder changes only the Status column and the notes under an item.

Status values: `READY`, `IN PROGRESS`, `BLOCKED`, `DONE`.

## Queue

| # | Item | Status |
| --- | --- | --- |
| Q1 | P2.0a: the LLUDP circuit in `ferroan-client`, as written in TASKS.md | READY |
| Q2 | Revise the P2.0 split (below) | READY |
| Q3 | Harness grid mode (the new row from Q2) | READY after Q2 merges |
| Q4 | P2.0b: seed capability and event queue | READY after Q2 merges |

Rows Q3 and later are briefed by Q2. Do not start P2.0c, P2.0e or P2.0g until Q2 has merged.

## Q1: P2.0a, the circuit

Go ahead with the row as written. If it is already in progress, finish it. Nothing in Q2 changes it.

## Q2: revise the P2.0 split

A docs-only PR to TASKS.md and [docs/ACTION_PLAN.md](docs/ACTION_PLAN.md) ("First login to a Ferroan region"), checked with `cargo xtask check --docs-only`. Re-letter the rows as you need, and record the old-to-new letter mapping in the PR body.

The split in #18 has a Ferroan region that is its own login server, and a harness with no grid for that region to join. D2 and the action plan say the first Ferroan region runs on the C# grid services. Make these changes:

1. **Add a harness grid-mode row, ahead of login.** `cargo xtask live` can run Tranquillity's GridServer (Robust) and a C# region as separate processes, on `127.0.0.1` with free ports, with the region's services reached over HTTP. Every existing scenario passes in grid mode. Standalone mode stays as it is. P2.0c, P2.0e and P2.0g depend on this row.
2. **Rewrite P2.0c: log in through C# Robust.** `ferroan-region` registers with the C# grid service, and serves the simulation endpoints that Robust's login service calls to place an agent. The region does not serve `login_to_simulator`. Before writing the row, read Tranquillity at `c85783e` to name the routes and payloads: the login service, the simulation service connector, and the agent handlers on the region side. Acceptance: `login: pass` through C# Robust onto the Ferroan region, with the region-to-Robust HTTP checked against a capture of a C# region doing the same.
3. **P2.0d: name the terrain source.** Load the heightmap a C# region saved, from the terrain table of Tranquillity's region store, unchanged. No Ferroan file format.
4. **P2.0e: appearance over HTTP.** It comes from the C# avatar service that Robust serves in grid mode, not from an in-process service.
5. **Split P2.0g in two.** First, a teleport inside the Ferroan region (`teleport: pass`, the same line as the C# region). Second, a teleport to a C# neighbour and back, with region-to-region traffic compared against a C# pair. Size the second from Tranquillity's entity transfer code. If it is more than two weeks of work, split it again.
6. **Update P2.0h** so the comparison runs in grid mode.

Keep each row to two weeks of work or less, with acceptance a bot can check. The Firestorm check stays with the maintainer (G2).

## Waiting on the maintainer

- G0: the Phase 0 Firestorm check, following [docs/MANUAL_CHECKS.md](docs/MANUAL_CHECKS.md), at the `c85783e` pin.
