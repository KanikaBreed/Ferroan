# Region admission and logout evidence

`exchanges.json` contains six HTTP probes captured from the local C# region at
Tranquillity `f67db54399c027fd74e5aefcb2193905b4dc1646`, plus four unambiguous
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
send `success:false` at this pin. Pin4's probe bodies match Pin3. The logout
`griduser-loggedout` position in this capture is `<128, 128, 26.033272>`; that
Z is the avatar position at logout, and the root-arrival `setposition` stays
`<128, 128, 25.960001>`.

Probe inputs contain only committed test identities and fixed bot device hashes.
Only the local test region name/UUID are substituted in the probe responses.
The observed loopback `your_ip` value remains because it is part of the answer.
The service exchanges preserve form-field order and XML bytes, replacing the
test agent/region UUIDs and generated session UUID with the same placeholders as
[the baseline login fixtures](../region-login/README.md). Source pairs are
selected only when no earlier request is outstanding and the following record
is its response. The missing-session Presence case uses a separate one-exchange
proxy to the same C# Robust backend: CreateAgent's concurrent inventory prefetch
can interleave with that lookup on the normal service proxy. Its explicit
`capture_source` records this distinction; ambiguous pairs are still rejected.
No password, live session ID or raw capture is committed.

Regenerating these fixtures requires `capture-udp.cap` in the same directory to
carry the pinned Tranquillity channel on the login circuit. The `source` label
written by the probe is not that proof. A Ferroan-region run is refused. Probe
requests carry `X-Ferroan-Probe` and are excluded from the selected service
exchanges. The missing-session query stays on its separate one-exchange proxy.

`AgentHandlers.cs`, `Scene.cs::VerifyUserPresence`, `PresenceServerPostHandler.cs`
and `GridUserServerPostHandler.cs` define these responses. The service calls come
from `PresenceServicesConnector.cs`, `GridUserServicesConnector.cs`,
`PresenceDetector.cs` and `ActivityDetector.cs` at the same revision.

The Rust login run also polls Robust until GridUser is offline and Presence has
no session for the test account, then requires a second login and logout and
checks both services again. Child-agent and non-login transfer support remains
outside the current Rust region: its capability refusals use C#'s 200 envelope,
but are not represented here as C# behavior. Firestorm remains pending.
