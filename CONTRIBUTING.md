# Contributing

Start with [HUMANS.md](HUMANS.md), [TASKS.md](TASKS.md) and [ARCHITECTURE.md](ARCHITECTURE.md). AI coding assistants must also read [AGENTS.md](AGENTS.md).

Work is offered as GitHub issues labelled [`agent-ready`](https://github.com/KanikaBreed/Ferroan/issues?q=is%3Aissue+is%3Aopen+label%3Aagent-ready). Claim one by commenting on it, open a draft PR within 3 days, and bring your own agent: [HUMANS.md](HUMANS.md#contributing-with-your-agent) has the setup and a prompt to give it. Each PR is reviewed against [docs/REVIEWING.md](docs/REVIEWING.md) before a maintainer merges it.

Keep contributions within the current phase of the [plan](docs/PLAN.md). Preserve compatibility with Tranquillity, follow the [porting rules](docs/PORTING.md) for licence headers and attribution, and include [evidence](docs/TESTING.md) for the behavior you changed.

Run `cargo xtask check` before submitting code. Use the pull request template and list every check that is still pending. Do not attach passwords, private captures or other people's data to a public report.
