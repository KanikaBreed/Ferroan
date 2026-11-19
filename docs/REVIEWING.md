# Reviewing outside pull requests

Outside contributors and their agents work on GitHub issues labelled `agent-ready` ([HUMANS.md](../HUMANS.md)). The project manager reviews each of their PRs first, against this list, and asks for changes on the PR. The maintainer approves and merges. Outside PRs never auto-merge. The project's own queue coder follows [WORK_QUEUE.md](../WORK_QUEUE.md) instead.

## Claims

- Comment on an issue to claim it. The manager adds `claimed`, and the claim is noted on the issue with its date.
- A claim lapses when no draft PR is open 3 days after it. The manager removes `claimed` and says so on the issue.
- A PR must name its issue (`Closes #n` or `Refs #n`). A PR with no issue is asked to open one first, unless it is a small fix to documentation or a typo.

## What the review checks

1. **Scope.** The PR does what its issue asks and nothing else. Changes outside the brief's "Where" are explained. It leaves TASKS.md and WORK_QUEUE.md alone; the manager updates them when it merges.
2. **Reference.** Ported behavior follows the C# at the revision pinned in [REFERENCES.md](REFERENCES.md), and the PR names the files followed. Behavior that differs from C# is either a recorded decision ([DECISIONS.md](DECISIONS.md)) or a bug in [TRANQUILLITY_BUGS.md](TRANQUILLITY_BUGS.md), never a silent change.
3. **Evidence.** Expected values come from C#: an oracle under `tools/*-oracle`, a capture from the live harness, or a captured exchange against a C# server. A Rust encoder checked only by its own decoder proves nothing (AGENTS.md §6). The reviewer re-runs the oracle `verify` the PR touches.
4. **Provenance and licence.** Each ported file carries its original notice, its C# source files and the revision ([PORTING.md](PORTING.md)). New sources are in [NOTICE.md](../NOTICE.md). No AGPL code and no material with unclear redistribution rights.
5. **Rust rules (AGENTS.md §2 and §6).** No `unsafe`. No `unwrap()` or `expect()` on outside input. Every size from outside is bounded. Structured errors. Async on tokio. No new native dependency. Any new crate has a compatible licence and a reason.
6. **Privacy.** Fixtures and captures come from test accounts on a test grid, with names, IP addresses and session ids removed or replaced. No passwords, grid credentials, tokens or personal paths in code, fixtures, logs or the PR text.
7. **Honest status.** `cargo xtask check` and CI pass. Checks only a viewer or a live grid can make are listed as pending, or recorded with the viewer, grid and revision, never claimed as passed without that.
8. **CI and tooling changes.** Changes to `.github/workflows`, `xtask` or the C# oracles get a closer read. They must not add secrets, network access beyond cloning pinned references and package registries, or `pull_request_target`.

## Outcome

- **Ready:** the manager says so on the PR, with what was checked. The maintainer approves and squash-merges it. The manager then updates the TASKS.md row and the issue.
- **Changes needed:** the manager lists each change on the PR. When the PR's brief was wrong, the manager fixes the issue rather than asking the contributor to guess.
- **Not taken:** when the work cannot be merged (wrong approach, or the plan changed), the manager says why on the PR and the issue, and what would be needed.
