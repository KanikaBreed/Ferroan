# Viewing the C# neighbour from a home root

Two Pin8 BasicPhysics regions run on the local grid. The resident logs in at (16, 128) on the neighbour. The observer logs in at (200, 128) at home. The harness copies the [Q65a SQLite store](../prim-updates/README.md) to the neighbour, assigns its prims to that region and shifts their group positions 110 m west. The known box is at (20, 128, 40), with stored UUID `65000000-0000-4000-8000-000000000001`. Both the resident and the box are within the observer's draw distance. The neighbour reopens the store with viewer object caching disabled.

The observer polls its home event queue, opens a second UDP socket with the same circuit code and session, requests the advertised seed, and answers the neighbour's region handshake. It sends forward controls on the home circuit. Both root viewers require a wire logout reply. The reference is OpenSim-Tranquillity `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`: `EntityTransferModule.EnableChildAgents` and `InformClientOfNeighbourAsync` arrange the admission and viewer events. `LLUDPServer.HandleUseCircuitCode` sends the child handshake. `ScenePresence.SendInitialData` sends the scene to the viewer after the handshake and seeds, and `CheckForSignificantMovement` sends child position updates.

## Files and replay

- `trace.json` records the two roots and the observer's child separately. Times are milliseconds since the earliest region UDP record. Each circuit lists the first occurrence of each selected message for that account and session. The proxy records the observer's child POST, position PUTs and DELETE in order. The extractor checks the original circuit code, session, destination and advertised seed against the admission. The JSON retains the child POST and PUT fields, including geometry, appearance, camera axes, throttles and transfer context. Account, session, circuit, inventory item, endpoint and seed values use consistent fixture aliases. The extractor pairs each request with its response, checks admission and update success and the closed agent's identity, and records the HTTP status. The DELETE credential stays in the private capture.
- `events.xml` holds the two LLSD events that the observer read from its home queue, in receipt order. The extractor normalizes the poll ID, endpoint, account and seed. The handle and region dimensions come from the simulator.
- `avatar.bin` and `prim.bin` hold the full, zero-decoded ObjectData blocks. Before selecting a block the extractor matches the sequence and entire message body from the observer's child socket against the C# neighbour's outgoing capture. It looks up the resident by account UUID and the box by store UUID. Their aliases are UUIDs 5 and 6 respectively. Nonzero owners get account aliases and nonempty name-value fields are redacted. The local IDs and region handle remain available for replay.

## Reproduce

From the repository root on Linux with the pinned references available:

```sh
mkdir -p .scratch/tmp .scratch/target
export TMPDIR="$PWD/.scratch/tmp" CARGO_TARGET_DIR="$PWD/.scratch/target"
export FERROAN_LIVE_PORT_BASE=26016 UseSharedCompilation=false MSBUILDDISABLENODEREUSE=1
flock -o /home/cinder/.local/share/ferroan-manager/live.lock cargo xtask live run child-view grid --physics basicphysics
cargo test -p xtask pin8_child_view
```

The live command prints the directory containing the redacted fixture under `.scratch/`. Timing, local IDs and sampled positions can vary between captures. The offline test replays the committed observation and checks its bytes.
