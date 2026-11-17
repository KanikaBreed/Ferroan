# Human contributor guide

Ferroan is built by volunteers. This guide covers how to pick up work, how to work with an AI agent, and what counts as done. Agents follow [AGENTS.md](AGENTS.md).

## Getting started

Clone the repository, install Rust through rustup and Python 3.11 or newer, and run `python3 scripts/check.py` (Windows: `py -3 scripts/check.py`). At this stage the workspace has one placeholder crate and no server to launch.

You will also want a clean checkout of [Tranquillity's `develop`](https://github.com/OpenSim-NGC/OpenSim-Tranquillity) beside this one. It is the reference for every behavior Ferroan ports.

For implementation work, use a task branch in its own worktree:

```sh
git fetch origin
git worktree add ../work/ferroan/llsd -b feat/llsd origin/main
```

Put temporary logs in `.scratch/`, which Git ignores.

## Picking a task

1. Find a `TODO` row in [TASKS.md](TASKS.md) in the current phase. Tasks are sized to finish in a few weeks; if one is bigger, split it first.
2. Mark it `IN PROGRESS` with your name and branch in a small PR, so two people don't take the same task.
3. Read the C# files the task ports and note the Tranquillity revision.
4. Port, test against captured C# behavior, and open a PR.

If you have to stop, say so in the task row and push what you have to the branch. Nobody should have to guess where a task stands.

## Working with an agent

1. State the behavior you want and any test grid you authorize it to use.
2. The agent reads the C# reference, scopes the change and updates TASKS.md.
3. It implements the change and runs the checks.
4. For anything a viewer or a live grid has to confirm, it gives you a build and short steps to check.
5. Record the viewer, grid, revision and result in the PR. Do not post passwords or other people's data.
6. Review the PR with its checks and evidence before merging.

A passing build does not mean a viewer can log in or that a script behaves as it did in C#. Those need their own evidence.

## Repository status

- README describes what exists now.
- ARCHITECTURE separates implemented crates from planned ones.
- TASKS tracks work and the evidence still missing.
- docs/PLAN holds the phases and gates; docs/DECISIONS the settled decisions.
- docs/PORTING holds the licence and attribution rules for ported code.

The repository is private for now. CI runs the checks on Linux, Windows and macOS.

## Contributions

Use English in code and GitHub artifacts. Keep reports focused, and remove grid addresses, account names, passwords and personal paths. See [bug reporting](docs/BUG_REPORTING.md) and [security](SECURITY.md). Never paste access tokens into issues, prompts or files. You can use your GitHub noreply address for commits.
