# Captures

Redacted session files belong here. `session.cap` is one `cargo xtask live run all` against the pinned Tranquillity region: login, walk, chat, rez, teleport, a texture upload, a library texture fetch and an inventory fetch. The chat text is `ferroan harness chat`. The raw capture is not stored.

`chat-debug.cap` contains the two outgoing `ChatFromSimulator` deliveries of `ferroan harness debug` from `cargo xtask live run chat grid` on Tranquillity Pin8 (`c401bceecd1684832ae8ba86a31b5ee01a4ded7b`). To reproduce the selection, keep the UDP-out records with that message and apply `ferroan_capture::redact_records`. Both original packets named the test speaker as source and owner. Redaction replaces the name and owner and preserves the public harness source ID, type, audibility, flags and message bytes.

The format, the recorder and the redaction rules are in [docs/CAPTURES.md](../../docs/CAPTURES.md). A file in this directory is a Ferroan encoding of the scrubbed datagrams. The raw capture, and the `ferroan-capture check` result on that raw file, stay outside Git.

The unit tests in `ferroan-capture` build a synthetic `UseCircuitCode` datagram. That datagram is not a session and it is not stored here.
