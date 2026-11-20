# Pin3 C# behavior observations

These fixtures execute KanikaBreed/OpenSim-Tranquillity `dev-build` `c70fee205f64ee8672a8d607cef6a733931da951` (Pin5) through [the Pin3 oracle](../../tools/pin3-oracle/Pin3Oracle.csproj). The directory name is the oracle that produced them. The source revision is recorded in each JSON file and the manifest. Pin5 regenerated them. The files contain actual return values, observed state and callback traces from C#; no Rust implementation generates their expected behavior.

`lifecycle.json` uses the pinned test helpers to construct real `Scene`, Presence and client instances. It invokes the actual login-arrival watcher for absent viewers, connected children, removal before root, completed root arrival, a later Presence session and zero timeout. GridUser is a test backend that records calls and persisted-position snapshots, so these observations establish which state C# changes and preserves; they do not claim to be a production Robust database capture. Fixed test UUIDs identify the fixture agent, sessions and saved region.

The close case invokes the actual `LLClientView` path with an intentionally throwing early callback. Its observed callback order, removal and logout-packet category show whether later cleanup still executes. The startup cases invoke actual `ActivityDetector` and `GridUserServicesConnector` with a controlled loopback HTTP responder. Their success/failure XML replies are injected test responses, not claims that a production or older Robust emitted those replies in a live run. The captured request form and returned behavior remain source-derived; ephemeral transport addresses and timing are not stored.

The pre-root endpoint case invokes the C# client replacement path. A repeat
from the same endpoint preserves the existing presence. At Pin5 (#437) a
different endpoint keeps the login watcher, the Presence session and the
online flag with the circuit and scene presence. This is recorded as TB11 in
[the bug ledger](../../docs/TRANQUILLITY_BUGS.md). Rust retains its existing
endpoint restriction and does not port the reconnect.
`csharp_pre_root_other_endpoint_keeps_watcher_and_online` requires
`pre_root_endpoint_reconnect.after` to show those states kept.

`trust.json` calls the actual `ControlPlaneAccess.HostOf` and trust evaluators. It supplies a deterministic DNS lookup and clock through the source's injectable constructor. The fixture covers host/address forms, both configuration sections and aliases, mapped and scoped IPv6, failed lookups, and lookup refresh timing. DNS observations describe the supplied test resolver, not public DNS availability. The Rust tests compare behavior with this trace.

Regenerate or verify using clean pinned references:

```sh
FERROAN_REF=/path/to/clean/reference cargo xtask pin3 generate
FERROAN_REF=/path/to/clean/reference cargo xtask pin3 verify
```

The driver validates the pin, clones reference sources into the task's ignored `.scratch/oracle-sources`, and builds into `.scratch/pin3-oracle/artifacts`. Reference checkouts are not built in or modified. Setup diagnostics and detailed oracle failures stay in scratch; committed observations contain fixed test data. CI verifies these observations alongside the five existing oracles and live C# and Rust runs. Viewer validation remains separate.
