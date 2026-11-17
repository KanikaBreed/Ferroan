# Captures

Redacted session files belong here. None are committed yet.

The format, the recorder and the redaction rules are in [docs/CAPTURES.md](../../docs/CAPTURES.md). A file in this directory is a Ferroan encoding of the scrubbed datagrams. The raw capture, and the `ferroan-capture check` result on that raw file, stay outside Git.

The unit tests in `ferroan-capture` build a synthetic `UseCircuitCode` datagram. That datagram is not a session and it is not stored here.
