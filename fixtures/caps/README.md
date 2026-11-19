# C# seed and event queue evidence

These HTTP bodies came from the local C# grid running Tranquillity
`40432e0f83d3802434f898a4e28687bd977d5cf5`. Regenerate with:

```sh
cargo xtask live build
cargo xtask live run caps grid
cargo xtask live caps-fixtures .scratch/live/grid
```

The dedicated `caps` scenario logs in as a generated local test account and
keeps its UDP session alive while probing HTTP. It requests only EventQueueGet
from the seed, repeats that request with an LLSD URI entry, checks a GET,
malformed POST and OPTIONS, probes LLSD binary parsing, drains startup events,
and waits for an empty poll.
It is separate from `all`, so ordinary scenarios do not gain a 58-second wait.
The same probe runs against Rust with
`cargo xtask live run caps grid --region ferroan`.

The recorded results are in `manifest.json`. Successful seed replies have no
XML declaration and use `text/html;charset=UTF-8`. GET returns 404 and malformed
LLSD returns 400, both with empty bodies and that same MIME type. OPTIONS
returns 200 with the three recorded CORS headers; it is handled before the
seed's POST-only guard. A URI entry naming EventQueueGet returns the same seed
map as a string entry.

The binary seed requests distinguish invalid base64 (`!`), unpadded base64,
unsupported `base16` with text, unsupported `base16` with an explicit empty
element, and a self-closing `base16` element. The first four return 400 with
empty bodies. C# accepts the self-closing element before inspecting its encoding;
that request returns 200 and an empty capability map. These are captured HTTP
results, in addition to the decoder oracle cases.

The manifest records the empty poll's elapsed milliseconds, measured between
the capture proxy's request and response records. It returned 502 after the
58-second hold, with `text/html` and zero body bytes. The selected nonempty
`events.response.xml` batch uses `application/xml` and contains EnableSimulator
and EstablishAgentCommunication for the C# neighbour. Its event id is preserved
in the response; the next empty poll acknowledges that id. Event ids are kept
unchanged so the capture documents the initial counter and its progression.

The generator changes only these identity/endpoint values, preserving every
other body byte and XML formatting:

- Seed and event queue URLs become documented placeholders under
  `192.0.2.1:9000`.
- The neighbouring simulator's IPv4 bytes, port and address string become
  `192.0.2.1:9001`.
- Its seed URL becomes a fixed placeholder and the event's agent UUID becomes
  `00000000-0000-4000-8000-000000000004`.

Body files have no added newline. `.body` files represent empty or deliberately
malformed bodies; `.xml` files retain the actual request/response XML. The
manifest retains method, normalized path, status, MIME types, CORS headers,
the captured Connection header, event URL shape and elapsed milliseconds.
`connection_bytes` records that the proxy rewrites hop headers to
`Connection: close` before C# reads the request, and that the capture stores
those forwarded request bytes. `response_connection` is the value C#
`HttpResponse` copied from that request
(`Source/OpenSim.Framework.Servers.HttpServer/OSHttpServer/HttpResponse.cs`
copies `request.Connection`). The event poll is the exception: its handler sets
Keep-Alive. Response bodies were not regenerated for this note.
Transport dates and other generated HTTP headers are
outside this comparison. Raw captures and real bearer URLs stay in `.scratch/`.
The generator rejects ambiguous request/response pairing, mixed capability
paths, compressed messages, unexpected bodies or MIME types, and missing
required exchanges before writing fixtures.

The sibling `capture-udp.cap` must contain the matching login sequence and the
exact pinned Tranquillity channel/version. The seed path must be that login
circuit's create-agent `caps_path` (`/CAPS/{caps_path}0000`, hyphenated as C#
registers it, or the same UUID without hyphens). A Rust region using the same
test account, name and UUID is rejected as a fixture source.

The source behavior is in these files at the revision above:

- [`BunchOfCaps.SeedCapRequest`](https://github.com/KanikaBreed/OpenSim-Tranquillity/blob/40432e0f83d3802434f898a4e28687bd977d5cf5/Source/OpenSim.Region.ClientStack.LindenCaps/BunchOfCaps/BunchOfCaps.cs)
- [`EventQueueGetModule`](https://github.com/KanikaBreed/OpenSim-Tranquillity/blob/40432e0f83d3802434f898a4e28687bd977d5cf5/Source/OpenSim.Region.ClientStack.LindenCaps/EventQueue/EventQueueGetModule.cs)
- [`EventQueueGetHandlers`](https://github.com/KanikaBreed/OpenSim-Tranquillity/blob/40432e0f83d3802434f898a4e28687bd977d5cf5/Source/OpenSim.Region.ClientStack.LindenCaps/EventQueue/EventQueueGetHandlers.cs)
- `Source/OpenSim.Framework.Servers.HttpServer/BaseHttpServer.cs`,
  `SimpleStreamHandler.cs` and `PollServiceHttpRequest.cs` (HTTP defaults,
  OPTIONS handling and held-poll responses).

The client tests serve the production `CapsRegistry::router()` and compare seed
and empty-poll responses with this independent C# evidence. The delivered-event test checks queue batch encoding using captured
events as input; it does not establish that the Rust region implements
crossings or produces those events. Viewer validation remains pending.
