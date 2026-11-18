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
| Q1 | P2.0a: the LLUDP circuit in `ferroan-client` | DONE (#20) |
| Q2 | P2.0b: seed capability and event queue | DONE (#21) |
| Q3 | P2.0c: harness grid mode | IN PROGRESS (feat/live-grid) |
| Q4 | P2.0d: login through C# Robust | READY after Q3 |

Q1: [PR #20](https://github.com/KanikaBreed/Ferroan/pull/20).

Q2: [PR #21](https://github.com/KanikaBreed/Ferroan/pull/21).

Q3: branch `feat/live-grid`.

Each item's scope and acceptance is its row in TASKS.md. The rows were revised on 18 November 2026: the first Ferroan region logs in through C# Robust and is not a login server, and the harness gains a grid mode first. Work from the rows as they stand on `main`, not from the earlier split in #18.

## Waiting on the maintainer

- G0: the Phase 0 Firestorm check, following [docs/MANUAL_CHECKS.md](docs/MANUAL_CHECKS.md), at the `c85783e` pin.
