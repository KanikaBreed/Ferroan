# Security and privacy

Ferroan has no releases and runs no service yet. Do not publish passwords, grid credentials, private captures or other users' data in issues.

While the repository is private, report sensitive findings directly to the maintainers rather than in an issue. When it becomes public, private vulnerability reporting will be enabled and this file will link to it.

The plan calls for bounded parsing of all network and file input, unsafe code confined to the two native-library crates, and redacted diagnostics. These are requirements, not claims that anything has passed a security review.

A virtual world server handles account passwords, session tokens, inventory and region content. Any Ferroan service must document what it stores, how it authenticates other services, and what it logs.
