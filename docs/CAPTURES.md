# Captures

A capture is one Tranquillity session, stored so a later test can decode every datagram and encode it again. The file format is shared by the C# recorder and `ferroan-capture`.

```text
magic:  FERROCAP1
record: kind u8 | time_ms u64 little-endian | len u32 little-endian | bytes
```

| Kind | Value | Bytes |
| --- | --- | --- |
| UDP in | 1 | The datagram the region received. At most 4096 bytes |
| UDP out | 2 | The datagram the region sent. At most 4096 bytes |
| HTTP request | 3 | The HTTP/1.1 request, headers and body. At most 1 MiB |
| HTTP response | 4 | The HTTP/1.1 response, headers and body. At most 1 MiB |

`time_ms` is Unix time in milliseconds. The socket address is not stored. A capture holds at most 1,000,000 records. Commit only reviewed, redacted evidence under `fixtures/`; full sessions belong in `fixtures/captures/`, and focused packet/HTTP evidence has its own fixture directories. `fixtures/captures/session.cap` is the redacted `cargo xtask live run all` session. The unit tests also build a synthetic `UseCircuitCode` datagram. That datagram is not a session.

## UDP

The recorder is [tools/capture/tranquillity-capture.patch](../tools/capture/tranquillity-capture.patch). It is applied to a scratch build of the Tranquillity revision in [REFERENCES.md](REFERENCES.md). It is not sent upstream, and it is not applied to `~/Git/reference`.

The patch adds `DatagramCapture.cs` and records in `OpenSimUDPBase`. Inbound bytes are taken in `AsyncBeginReceive` after the length is known and before `PacketReceived`. Outbound bytes are taken in `SyncSend` immediately before `SendTo`. A packet that the pending queue injects by calling `PacketReceived` directly is not written a second time. A datagram of length 0 or longer than 4096 is skipped. Each record is flushed. `StopInbound` closes the file. A write error closes the file and logs the exception type. The path is not logged.

`[ClientStack.LindenUDP] CaptureFile` is the path. An absent or empty value leaves the server unchanged. The same patch also adds `ServiceProbeConnectorModule`, loaded only when `[ClientStack.LindenUDP] ServiceProbe` is true (the default is false). That module hosts the region's in-process Presence and GridUser handlers for the harness probe. It does not change datagram capture.

Build and apply it in a scratch clone. Do not build inside the reference checkout.

```sh
git clone --shared ~/Git/reference/OpenSim-Tranquillity ~/Git/work/ferroan/tranquillity-capture
git -C ~/Git/work/ferroan/tranquillity-capture apply "$PWD/tools/capture/tranquillity-capture.patch"
```

Then set the key in the region's INI and start that build:

```ini
[ClientStack.LindenUDP]
    CaptureFile = /tmp/session.cap
```

`cargo xtask live build` applies the patch and builds the pinned C# server in `.scratch/live/`. The [Linux live workflow](../.github/workflows/live.yml) starts those regions with `CaptureFile` set, runs standalone and grid scenarios, and requires the resulting UDP captures to pass check, redaction and replay. The committed session below is also replayed by the ordinary CI jobs.

## HTTP

`ferroan-capture http-proxy` is a logging reverse proxy for plain HTTP. Point a region's service URL at `--listen` and set `--upstream` to the real service. The same proxy can sit in front of a CAPS endpoint.

```sh
cargo run -p ferroan-capture -- http-proxy \
    --listen 127.0.0.1:9000 \
    --upstream http://127.0.0.1:8003 \
    --out /tmp/http.cap
```

`--upstream` is `http://` followed by an IP and port. A name is not resolved. `https://` is rejected: this proxy does not intercept TLS. A request or response that uses `Transfer-Encoding` is refused. A request without `Content-Length` is stored with an empty body. A response without `Content-Length` is read until the server closes, up to 1 MiB.

An exchange failure makes the CLI exit unsuccessfully and fails a harness run. Errors include the request/response phase and byte counts, without printing message contents. A clean TCP close before any request bytes is an unused connection and records no exchange; partial HTTP headers or bodies still fail. Up to 16 interim `1xx` responses are skipped before the final response under the same reply deadline. Harness shutdown drains accepted exchanges and checks their failure counts before reporting success.

## Replay before redaction

On the raw file, before anything is replaced:

```sh
cargo run -p ferroan-capture -- check /tmp/session.cap
```

`check` and `replay` both call `reencode_datagram` on every UDP record and require the same bytes. HTTP records are skipped by that check. The output line is `replay: N records, udp round-trip matched`. A mismatch names the record index, the direction, the message name and the capture timestamp, and stops. `check` also writes `reencode-mismatch.log` beside the capture: the index, direction, message name, timestamp, and the original and re-encoded datagrams in hex. The live harness writes the same file at `.scratch/live/diag/reencode-mismatch.log`, which the live job uploads with the other diagnostics. That equality is the wire proof. Keep the raw file out of Git.

## Redaction

```sh
cargo run -p ferroan-capture -- redact --in /tmp/session.cap --out fixtures/captures/session.cap
```

`redact` runs the same UDP check first. A record that does not re-encode is not rewritten. UDP records are then `scrub_datagram`:

| Field | Replacement |
| --- | --- |
| `IPADDR` | `127.0.0.1`. The port stays |
| UUID whose name contains `session` | `00000000-0000-0000-0000-000000000002` |
| UUID whose name contains `agentid`, `userid`, `ownerid`, `creatorid`, `abuserid` or `reporterid`, and `ID` inside a circuit block | `00000000-0000-0000-0000-000000000001` |
| U32 named `Code`, or whose name contains `circuitcode` | `1` |
| Variable field named Name, FromName, FromAgentName, FirstName, LastName, UserName, NameValue or ObjectOwner | `Redacted` |
| Variable field whose name contains password, passwd, secret, token or email | `Redacted` |

Object UUIDs stay, including `SourceID`, which is an agent id on some chat and an object id on others. Chat `Message` text stays, so read the redacted file before committing it. An object `Name` becomes `Redacted`. `OwnerID` and `LastOwnerID` become the agent placeholder. The placeholders are fixed values. They are not a hash of the original.

HTTP `Authorization`, `Cookie`, `Set-Cookie` and `Proxy-Authorization` values become `Redacted`. Other header lines, and a body whose `Content-Type` contains text, xml, json, form or llsd, have dotted IPv4 addresses replaced with `127.0.0.1`. An XML-RPC `<string>`, `<int>` or `<i4>` that follows `<name>password</name>`, or the same shape for passwd, token, secret, first, last, firstname, lastname, session, session_id, secure_session_id, agent_id, circuit_code or seed_capability, becomes `Redacted`. An LLSD `<uuid>` or `<string>` that follows `<key>owner_id</key>`, or the same shape for agent_id, creator_id, last_owner_id, session_id or secure_session_id, becomes `Redacted`. An `asset_id` stays. A body with another content type is copied unchanged. A missing `Content-Type` is treated as text.

The committed UDP bytes are the encoding of the scrubbed message. They round-trip through `reencode_datagram`. They are not the original datagram. The equality that matters for the wire is the `check` on the raw file, done before `redact`.

## The committed session

`fixtures/captures/session.cap` is a redacted `cargo xtask live run all`. The bot logged in, walked, said `ferroan harness chat`, rezzed a prim, teleported, uploaded a texture, fetched the library kelp texture and fetched inventory, then logged out. Both bots' `UseCircuitCode` packets carry circuit code 1 and the session and agent placeholders. The raw file was checked with `ferroan-capture check` before redaction; that file is not in Git. `the_harness_session_replays` runs `replay` on the committed bytes.

A later session can replace it. Record with `CaptureFile` on a scratch build, run `check` on the raw file, run `redact`, read the chat text, and commit only the redacted file. A message that fails `replay` becomes a task. It is not dropped quietly.

## Neighbour teleport

`fixtures/neighbour-teleport/` is a redacted `cargo xtask live run neighbour grid` at pin `1d7abb8337`. Pin7's crossing change (#494) only stops a region resending a crossing to an avatar that crossed out and came back; this scenario teleports and never crosses, so the files stay this capture. Both regions are the pinned C# server. The home region writes `capture-udp.cap`. The neighbour writes `capture-udp-neighbour.cap`. Region-to-region HTTP is `proxy-home.cap` and `proxy-neighbour.cap`. The raw files were checked with `ferroan-capture check` before redaction. Session ids, circuit codes, capability paths and the `CloseAgent` `auth` query use the same placeholders as the login fixtures. `neighbour_teleport_fixture_replays_and_compares` replays the four files and checks the teleport fields. The live neighbour scenario runs that same compare on a fresh capture.
