# Security and privacy

Ferroan has no releases and runs no service yet. Do not publish passwords, grid credentials, private captures or other users' data in issues.

Report security problems privately through GitHub: [Report a vulnerability](https://github.com/KanikaBreed/Ferroan/security/advisories/new) (private vulnerability reporting is on). Do not open a public issue for them.

The plan calls for bounded parsing of all network and file input, unsafe code confined to the two native-library crates, and redacted diagnostics. These are requirements, not claims that anything has passed a security review.

A virtual world server handles account passwords, session tokens, inventory and region content. Any Ferroan service must document what it stores, how it authenticates other services, and what it logs.
