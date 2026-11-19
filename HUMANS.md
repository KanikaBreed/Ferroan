# Human contributor guide

Ferroan is built by volunteers. This guide covers how to pick up work, how to work with an AI agent, and what counts as done. Agents follow [AGENTS.md](AGENTS.md).

## Getting started

Clone the repository, install Rust through rustup, and run `cargo xtask check`.

Most work also needs the C# reference and the tools that run it:

- **The .NET 10 SDK.** The C# oracles and the live harness build and run Tranquillity.
- **Clean clones of the pinned references.** Clone them into one folder and point `FERROAN_REF` at it. Take each repository URL and revision from [docs/REFERENCES.md](docs/REFERENCES.md). The Tranquillity pin may be on the maintainer's fork rather than OpenSim-NGC ([D21](docs/DECISIONS.md)), so use the URL in the table. Never build in or edit these clones.

  ```sh
  mkdir -p ~/ferroan-ref && cd ~/ferroan-ref
  git clone <Tranquillity URL from REFERENCES.md> OpenSim-Tranquillity
  git -C OpenSim-Tranquillity checkout --detach <Tranquillity revision>
  git clone https://github.com/OpenSim-NGC/libopenmetaverse.git
  git -C libopenmetaverse checkout --detach <libopenmetaverse revision>
  export FERROAN_REF=~/ferroan-ref
  ```

  When the pin moves, fetch and check out the new revision.

For implementation work, use a task branch in its own worktree:

```sh
git fetch origin
git worktree add ../work/ferroan/llsd -b feat/llsd origin/main
```

Put temporary logs in `.scratch/`, which Git ignores.

## Two ways work gets done

- **The project's own coder agent** works through [WORK_QUEUE.md](WORK_QUEUE.md), one item at a time, on the main line of the plan. The maintainers start and run it.
- **Everyone else** works on GitHub issues labelled [`agent-ready`](https://github.com/KanikaBreed/Ferroan/issues?q=is%3Aissue+is%3Aopen+label%3Aagent-ready). Each one has a complete brief: the C# files to follow, where the code goes, what proves it, and what is out of scope. The same tasks are also on the queue, and the coder does not wait for contributors: when it reaches one whose issue is not `claimed`, it claims the issue itself and says so there. Claim an issue before the coder reaches it and the coder skips it, so the two never take the same work.

## Picking a task

1. Pick an open issue labelled `agent-ready` and not `claimed`. Size labels are a rough guide: `size-s` is about one agent session, `size-m` a few, and `size-l` several.
2. Comment on it to claim it. The manager adds `claimed`. Claim one issue at a time.
3. Open a draft PR within 3 days and push as you go. A claim with no draft PR after 3 days lapses, and the issue goes back to `agent-ready`.
4. If you have to stop, say so on the issue and push what you have. Nobody should have to guess where a task stands.
5. If the brief is wrong, too big, or blocked by something else, say so on the issue before changing the scope.

Questions about setup, the plan or the code go in [Discussions](https://github.com/KanikaBreed/Ferroan/discussions) (Q&A). Ideas that don't fit an existing task go in its Ideas category.

Want to take on something with no issue yet? Ask in [Discussions](https://github.com/KanikaBreed/Ferroan/discussions) first. The manager writes the brief, so the work fits the plan and doesn't collide with the queue.

## Contributing with your agent

1. Set up as above, and claim an issue.
2. Give your agent this prompt, with the issue number filled in:

   ```text
   You are contributing to Ferroan (https://github.com/KanikaBreed/Ferroan) on issue #<N>, which I have claimed.
   Read AGENTS.md and the files it lists, then the issue. Work only on that issue, in a git worktree on a branch
   named after it. You are not the project's queue coder: do not take WORK_QUEUE.md items, and do not edit
   TASKS.md or WORK_QUEUE.md. Port from the C# at the revision pinned in docs/REFERENCES.md (FERROAN_REF points at
   clean clones), and prove behavior with fixtures or captures from the C# server, never with Rust round trips
   alone. Run `cargo xtask check`. Open a PR from my fork with `Closes #<N>`, filling in the PR template,
   including what is still pending. If the issue is wrong or too big, stop and tell me what to post on the issue.
   ```

3. Read the PR yourself before you mark it ready for review. You are responsible for what your agent submits.
4. For anything only a viewer or a live grid can confirm, record it in the PR as pending, or record what you saw: the viewer, grid, revision and result. Never post passwords, grid credentials or other people's data.

A passing build does not mean a viewer can log in or that a script behaves as it did in C#. Those need their own evidence.

## Review and merging

The manager reviews each outside PR first, against [docs/REVIEWING.md](docs/REVIEWING.md), and asks for changes on the PR. The maintainer approves and merges. Outside PRs never auto-merge. CI runs on PRs from forks with no secrets. A first-time contributor's workflows wait for a maintainer to approve the run.

## Repository status

- README describes what exists now.
- ARCHITECTURE separates implemented crates from planned ones.
- TASKS tracks work and the evidence still missing.
- docs/PLAN holds the phases and gates; docs/DECISIONS the settled decisions.
- docs/PORTING holds the licence and attribution rules for ported code.

The repository is public. CI runs the checks on Linux, Windows and macOS.

## Contributions

Use English in code and GitHub artifacts. Keep reports focused, and remove grid addresses, account names, passwords and personal paths. See [bug reporting](docs/BUG_REPORTING.md) and [security](SECURITY.md). Never paste access tokens into issues, prompts or files. You can use your GitHub noreply address for commits.
