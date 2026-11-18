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
| Q3 | P2.0c: harness grid mode | DONE (#22) |
| Q4 | Pin2: move the Tranquillity pin to `develop` `8583218` | DONE (#24) |
| Q5 | P0.10f: fix `live assets` and add `live up --assets ferroan` | DONE (#25) |
| Q6 | P2.0d: login through C# Robust | IN PROGRESS (feat/region-login) |
| Q7 | P2.0b2: the event queue as Tranquillity answers it | READY after Q6 |
| Q8 | P2.0e: terrain from the C# region store | READY after Q7 |
| Q9 | P2.0f: appearance from the C# avatar service | READY after Q8 |
| Q10 | P2.0g: local chat | READY after Q9 |
| Q11 | P2.0h: teleport inside the Ferroan region | READY after Q10 |
| Q12 | P2.0i: teleport to the C# neighbour and back | READY after Q11 |
| Q13 | P2.0j: grid-mode report comparison and the first-login checklist | READY after Q12 |

Q1: [PR #20](https://github.com/KanikaBreed/Ferroan/pull/20).

Q2: [PR #21](https://github.com/KanikaBreed/Ferroan/pull/21).

Q3: [PR #22](https://github.com/KanikaBreed/Ferroan/pull/22).

Q4 to Q7 were added on 18 November 2026. They come before the rest of P2.0 because `~/Git/reference` now follows `develop` `8583218`, and `cargo xtask live build` refuses a reference checkout that is not at the pin.

Q6: the `feat/region-login` worktree holds only status edits for this item, made before it was moved. Discard them and start from `main` once Q5 is done.

Q12: size it from the C# first, and set it to `BLOCKED` with a proposed split if it is more than two weeks of work.

Each item's scope and acceptance is its row in TASKS.md. The rows were revised on 18 November 2026: the first Ferroan region logs in through C# Robust and is not a login server, and the harness gains a grid mode first. Work from the rows as they stand on `main`, not from the earlier split in #18.

## Waiting on the maintainer

- Nothing. G0 passed on 18 November 2026 at `c85783e` (see its row in TASKS.md).
