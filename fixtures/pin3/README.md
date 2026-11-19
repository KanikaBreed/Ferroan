# Pin3 C# behavior observations

These fixtures execute Tranquillity `40432e0f83d3802434f898a4e28687bd977d5cf5`, the maintainer's `dev-build` tag `ferroan-pin3`, through [the Pin3 oracle](../../tools/pin3-oracle/Pin3Oracle.csproj). The source revision is recorded in each JSON file and the manifest. The files contain actual return values, observed state and callback traces from C#; no Rust implementation generates their expected behavior.

`lifecycle.json` uses the pinned test helpers to construct real `Scene`, Presence and client instances. It invokes the actual login-arrival watcher for absent viewers, connected children, removal before root, completed root arrival, a later Presence session and zero timeout. GridUser is a test backend that records calls and persisted-position snapshots, so these observations establish which state C# changes and preserves; they do not claim to be a production Robust database capture. Fixed test UUIDs identify the fixture agent, sessions and saved region.

The close case invokes the actual `LLClientView` path with an intentionally throwing early callback. Its observed callback order, removal and logout-packet category show whether later cleanup still executes. The startup cases invoke actual `ActivityDetector` and `GridUserServicesConnector` with a controlled loopback HTTP responder. Their success/failure XML replies are injected test responses, not claims that a production or older Robust emitted those replies in a live run. The captured request form and returned behavior remain source-derived; ephemeral transport addresses and timing are not stored.

The pre-root endpoint case invokes the merged C# client replacement path. A
repeat from the same endpoint preserves the existing presence. A different
endpoint restores the circuit and replaces the scene presence, but removes the
login watcher and marks the services offline. This is recorded as TB11 in
[the bug ledger](../../docs/TRANQUILLITY_BUGS.md), not asserted as desirable
reconnect behavior. Rust retains its existing endpoint restriction pending the
maintainer's disposition.

`trust.json` calls the actual `ControlPlaneAccess.HostOf` and trust evaluators. It supplies a deterministic DNS lookup and clock through the source's injectable constructor. The fixture covers host/address forms, both configuration sections and aliases, mapped and scoped IPv6, failed lookups, and lookup refresh timing. DNS observations describe the supplied test resolver, not public DNS availability. The Rust tests compare behavior with this trace.

Regenerate or verify using clean pinned references:

```sh
FERROAN_REF=/path/to/clean/reference cargo xtask pin3 generate
FERROAN_REF=/path/to/clean/reference cargo xtask pin3 verify
```

The driver validates the pin, clones reference sources into the task's ignored `.scratch/oracle-sources`, and builds into `.scratch/pin3-oracle/artifacts`. Reference checkouts are not built in or modified. Setup diagnostics and detailed oracle failures stay in scratch; committed observations contain fixed test data. CI verifies these observations alongside the five existing oracles and live C# and Rust runs. Viewer validation remains separate.
