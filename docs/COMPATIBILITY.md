# Compatibility

Ferroan must work on grids where the rest is still Tranquillity ([D2](DECISIONS.md)), so the C# server is the specification. A Rust piece replaces a C# piece only after it has been checked against what the C# piece does.

## What must match

| Interface | Used by | Must match |
| --- | --- | --- |
| Viewer protocol | Firestorm, Aurora and other SL-style viewers | LLUDP message template, acks and throttles, CAPS with LLSD, the event queue, mesh and texture fetch. The template and framing choices are in [PROTOCOL.md](PROTOCOL.md) |
| Login | Viewers and grid websites | XML-RPC `login_to_simulator` and its response fields |
| Region to Robust | C# regions and services | The service connectors' HTTP and XML forms. The asset routes are the first of these, in `ferroan-services` |
| Hypergrid | Other OpenSim grids | Gatekeeper, user agent, HG inventory and asset services, universal user identifiers |
| Region to region | Neighbouring C# regions | Agent and object transfer on crossings and teleports |
| Storage | Existing databases | Tranquillity's MySQL, PostgreSQL and SQLite schemas and applied migrations |
| Archives | OAR and IAR files | The C# XML serialization of objects, shapes, inventory and terrain |
| Scripts | Residents' scripts | Function results, event order and quirks, as the SLua engine plan's conformance suite checks them |
| Configuration | Operators | Tranquillity's INI sections and keys |

## How it is checked

- **Captured traffic.** Record C# sessions on a test grid, redact them, and replay them against the Rust piece. The recorder and the file format are in [CAPTURES.md](CAPTURES.md). Responses must match apart from fields known to vary, such as timestamps and session IDs. The redacted [session.cap](../fixtures/captures/session.cap) is committed, with focused circuit, arrival, CAPS and login fixtures under `fixtures/`.
- **Side by side.** Run each Rust service behind a live test grid with C# regions, with a switch back to C#.
- **Viewers.** Scripted Aurora logins and actions, and checks by hand in Firestorm.
- **Archives.** Load OAR and IAR files into both servers, save them again, and compare.
- **Existing tests.** Tranquillity's C# tests are read as specifications, and those that test behavior are ported.

Differences from Tranquillity are allowed only as recorded decisions, never by accident.
