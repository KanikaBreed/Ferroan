# Reporting bugs

Ferroan implements an asset-service process and a limited region that logs in through C# Robust. Reports can cover those processes, the live harness, builds, fixtures or documentation; [ARCHITECTURE.md](../ARCHITECTURE.md) lists the implemented scope.

A useful report gives:

- the commit hash,
- the operating system and Rust version (`rustc --version`),
- the command you ran, what you expected and what happened,
- a short excerpt of the output.

For a service, region or harness report, also give the client and its version, whether the grid mixes Rust and C# pieces, and the Tranquillity revision of any C# pieces.

Remove passwords, grid addresses, account names, session IDs and personal paths before posting. Report security problems as [SECURITY.md](../SECURITY.md) describes, not in an issue.
