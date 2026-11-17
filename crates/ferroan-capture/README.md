# ferroan-capture

Record, redact and replay a Tranquillity session. The file format, the commands and the session steps are in [docs/CAPTURES.md](../../docs/CAPTURES.md).

`http-proxy` appends each plain HTTP request and response. `check` and `replay` require every UDP datagram to decode and encode back to the same bytes. `redact` runs that check, then replaces session, agent, circuit, name and address fields with fixed placeholders.

UDP recording is [tools/capture/tranquillity-capture.patch](../../tools/capture/tranquillity-capture.patch), applied only to a scratch build of Tranquillity. This crate does not intercept HTTPS. No session capture is committed yet. The tests use a synthetic `UseCircuitCode` datagram and one local HTTP exchange.
