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

`time_ms` is Unix time in milliseconds. The socket address is not stored. A capture holds at most 1,000,000 records. Commit only a redacted file, under `fixtures/captures/`. No session is in the tree yet. The unit tests build a synthetic `UseCircuitCode` datagram. That datagram is not a session.

## UDP

The recorder is [tools/capture/tranquillity-capture.patch](../tools/capture/tranquillity-capture.patch). It is applied to a scratch build of Tranquillity `713857f10b8e91a639a967d4354a4682b080793c`. It is not sent upstream, and it is not applied to `~/Git/reference`.

The patch adds `DatagramCapture.cs` and records in `OpenSimUDPBase`. Inbound bytes are taken in `AsyncBeginReceive` after the length is known and before `PacketReceived`. Outbound bytes are taken in `SyncSend` immediately before `SendTo`. A packet that the pending queue injects by calling `PacketReceived` directly is not written a second time. A datagram of length 0 or longer than 4096 is skipped. Each record is flushed. `StopInbound` closes the file. A write error closes the file and logs the exception type. The path is not logged.

`[ClientStack.LindenUDP] CaptureFile` is the path. An absent or empty value leaves the server unchanged.

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

`dotnet build` of `Source/OpenSim.Region.ClientStack.LindenUDP/OpenSim.Region.ClientStack.LindenUDP.csproj` with this patch applied succeeded on Linux (0 warnings, 0 errors) against that revision. A region process has not been started with `CaptureFile` set.

## HTTP

`ferroan-capture http-proxy` is a logging reverse proxy for plain HTTP. Point a region's service URL at `--listen` and set `--upstream` to the real service. The same proxy can sit in front of a CAPS endpoint.

```sh
cargo run -p ferroan-capture -- http-proxy \
    --listen 127.0.0.1:9000 \
    --upstream http://127.0.0.1:8003 \
    --out /tmp/http.cap
```

`--upstream` is `http://` followed by an IP and port. A name is not resolved. `https://` is rejected: this proxy does not intercept TLS. A request or response that uses `Transfer-Encoding` is refused. A request without `Content-Length` is stored with an empty body. A response without `Content-Length` is read until the server closes, up to 1 MiB.

## Replay before redaction

On the raw file, before anything is replaced:

```sh
cargo run -p ferroan-capture -- check /tmp/session.cap
```

`check` and `replay` both call `reencode_datagram` on every UDP record and require the same bytes. HTTP records are skipped by that check. The output line is `replay: N records, udp round-trip matched`. A mismatch names the record index and stops. That equality is the wire proof. Keep the raw file out of Git.

## Redaction

```sh
cargo run -p ferroan-capture -- redact --in /tmp/session.cap --out fixtures/captures/session.cap
```

`redact` runs the same UDP check first. A record that does not re-encode is not rewritten. UDP records are then `scrub_datagram`:

| Field | Replacement |
| --- | --- |
| `IPADDR` | `127.0.0.1`. The port stays |
| UUID whose name contains `session` | `00000000-0000-0000-0000-000000000002` |
| UUID whose name contains `agentid` or `userid`, and `ID` inside a circuit block | `00000000-0000-0000-0000-000000000001` |
| U32 named `Code`, or whose name contains `circuitcode` | `1` |
| Variable field named Name, FromName, FirstName, LastName, UserName or NameValue | `Redacted` |
| Variable field whose name contains password, passwd, secret, token or email | `Redacted` |

Object UUIDs stay. Chat `Message` text stays, so read the redacted file before committing it. An object `Name` becomes `Redacted`. The placeholders are fixed values. They are not a hash of the original.

HTTP `Authorization`, `Cookie`, `Set-Cookie` and `Proxy-Authorization` values become `Redacted`. Other header lines, and a body whose `Content-Type` contains text, xml, json, form or llsd, have dotted IPv4 addresses replaced with `127.0.0.1`. An XML-RPC `<string>` that follows `<name>password</name>`, or the same shape for passwd, token, secret, first, last, firstname, lastname, session or session_id, becomes `Redacted`. A body with another content type is copied unchanged. A missing `Content-Type` is treated as text.

The committed UDP bytes are the encoding of the scrubbed message. They round-trip through `reencode_datagram`. They are not the original datagram. The equality that matters for the wire is the `check` on the raw file, done before `redact`.

## The session still to record

No Aurora session has been recorded. The next session is a local Tranquillity region with SQLite, a test account, and the patch applied:

1. Apply the patch to a scratch build and set `CaptureFile`.
2. Point the region's service URLs at `ferroan-capture http-proxy` when those calls should be kept.
3. Log in with Aurora. Record the viewer revision.
4. Walk, chat, rez a prim, edit that prim, teleport within the region, and log out.
5. Run `check` on the raw capture and keep that result.
6. Run `redact`. Read the chat text. Commit only the redacted file under `fixtures/captures/`.
7. Run `replay` on the committed file. A message that fails becomes a task. It is not dropped quietly.

Until that file exists, P0.5 has no captured-traffic fixture. The synthetic `UseCircuitCode` test shows the scrub and the round trip. It does not stand in for the session.
