# Region admission and logout evidence

`exchanges.json` contains six HTTP probes captured from the local C# region at
Tranquillity `858321883f5487a5d192d36ea9d7b90fa468310c`, plus four unambiguous
Presence/GridUser service exchanges from that login, logout and probe run.

```sh
cargo xtask live build
cargo xtask live run login grid
cargo xtask live region-parity-fixtures .scratch/live/grid
cargo xtask live run login grid --region ferroan
```

The probes use the committed CreateAgent and QueryAccess request shapes. They
cover an unknown session, missing destination, numeric-string versions, a
non-array features value, incompatible versions and legacy version 0.7. POSTs
send `application/json` with `X-Content-Encoding: gzip`, as the C# connector does.
Each entry records the actual status, content type and parsed JSON response.
JSON member whitespace/order is not compared; numeric precision is preserved.
The 0.3 negotiation exposes the full widened single-precision value
`0.30000001192092896`. A failed CreateAgent response omits `success`; it does not
send `success:false` at this pin.

Probe inputs contain only committed test identities and fixed bot device hashes.
Only the local test region name/UUID are substituted in the probe responses.
The observed loopback `your_ip` value remains because it is part of the answer.
The service exchanges preserve form-field order and XML bytes, replacing the
test agent/region UUIDs and generated session UUID with the same placeholders as
[the baseline login fixtures](../region-login/README.md). Source pairs are
selected only when no earlier request is outstanding and the following record
is its response. No password, live session ID or raw capture is committed.

`AgentHandlers.cs`, `Scene.cs::VerifyUserPresence`, `PresenceServerPostHandler.cs`
and `GridUserServerPostHandler.cs` define these responses. The service calls come
from `PresenceServicesConnector.cs`, `GridUserServicesConnector.cs`,
`PresenceDetector.cs` and `ActivityDetector.cs` at the same revision.

The Rust login run also polls Robust until GridUser is offline and Presence has
no session for the test account, then requires a second login and logout and
checks both services again. Child-agent and non-login transfer support remains
outside the current Rust region: its capability refusals use C#'s 200 envelope,
but are not represented here as C# behavior. Firestorm remains pending.
