# ferroan-capture

Record, redact and replay a Tranquillity session. The file format, the commands and the session steps are in [docs/CAPTURES.md](../../docs/CAPTURES.md).

`http-proxy` appends each plain HTTP request and response. `check` and `replay` require every UDP datagram to decode and encode back to the same bytes. `redact` runs that check, then replaces session, agent, circuit, name and address fields with fixed placeholders.

`http-proxy --reply-timeout-secs 70` gives an upstream reply up to 70 seconds, including its body; the default is 70 and the allowed range is 1–300 seconds. This accommodates Tranquillity's 58-second event-queue hold. Request reads, connects and writes remain bounded at five seconds. A timeout fails the exchange rather than returning a partial body. The proxy handles one exchange per connection and forwards `Connection: close` to both endpoints. Captures retain the original request and response headers received from those endpoints, before that hop-specific rewrite.

UDP recording is [tools/capture/tranquillity-capture.patch](../../tools/capture/tranquillity-capture.patch), applied only to a scratch build of Tranquillity. This crate does not intercept HTTPS. The redacted [session.cap](../../fixtures/captures/session.cap) is committed and checked by `the_harness_session_replays`, alongside synthetic packet and local HTTP proxy tests. The [Linux live workflow](../../.github/workflows/live.yml) records fresh C# sessions and requires check, redaction and replay to pass; raw captures stay under `.scratch/`.

Any failed exchange makes `http-proxy` exit unsuccessfully, including timeout, unsupported chunked framing, oversize messages and early EOF. The harness retains shared failure counts and drains accepted exchanges before reporting success. Up to 16 interim `1xx` responses are skipped under the same total reply deadline; the final response is captured. Concurrent exchanges are bounded at 256.

A socket closed cleanly before sending any HTTP bytes contains no exchange. Partial requests still fail, and diagnostics report phase and byte counts without message contents.
