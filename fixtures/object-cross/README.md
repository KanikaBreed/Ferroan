# Riderless linkset crossing

The source is Tranquillity Pin8 (`c401bceecd1684832ae8ba86a31b5ee01a4ded7b`). The two grid regions run in separate C# processes with BasicPhysics. Home loads a fresh copy of the [two-part prim fixture](../prim-updates/README.md). Each region has its own connected test viewer. Neither viewer rides the object.

At this pin, `LLClientView.HandleMultipleObjUpdate` handles the edit. `SceneObjectGroup.CrossAsyncCore` takes the riderless path to `EntityTransferModule.CrossPrimGroupIntoNewRegion`. `SimulationServiceConnector.CreateObject` posts the XML, and `ObjectHandlers.RespondWithBool` writes the answer.

The home viewer grants everyone Move and waits for the root's permission reply. Each viewer sends one reliable `MultipleObjectUpdate` for its region's root local ID. Type 9 moves the whole linkset. The first edit targets (264, 128, 40) in home coordinates, the second targets (−8, 128, 40) in neighbour coordinates. The viewers observe the root at (8, 128, 40) on the neighbour and (248, 128, 40) on home.

## Reproduce

Build the pinned server with `cargo xtask live build`. On the shared development host, run the following from the worktree with `TMPDIR` and `CARGO_TARGET_DIR` under its `.scratch/`.

```sh
FERROAN_LIVE_PORT_BASE=27040 UseSharedCompilation=false \
  flock /home/cinder/.local/share/ferroan-manager/live.lock \
  cargo xtask live run object-cross grid --physics basicphysics
```

The last report line names the extracted directory. To repeat extraction from the same stopped grid, use `cargo xtask live object-cross-fixtures <grid-capture-directory>`. Raw proxy and UDP captures stay in scratch.

## Files and replay

The eastbound and westbound HTTP files contain the object POST and its paired answer as seen by the destination proxy. The request body includes the original XML2 in `sog` and the inventory item XML in `extra`. The seeded linkset has no script state. Extraction checks both region proxy streams for unexpected access queries before selecting the object exchanges.

In this run, each proxy saw one avatar login access query before its object POST. Both object replies were HTTP 200 with the four-byte body `True`. The captured `modified` field is the JSON boolean true in both directions.

The two viewer files use the repository's FERROCAP1 format. They retain the group edit and object updates for the root and linked child on that viewer's circuit. Home recorded five full-update datagrams for this linkset and the outbound edit. The neighbour recorded one full-update datagram and the return edit. Neither capture contains a linkset kill or terse update. Both raw streams identify the connected viewer and the Pin8 build in its movement-complete packet. Packet order, local IDs and parent references are evidence. Other objects and unrelated traffic in mixed packets are removed. The edit's session ID is cleared, test-account IDs are replaced by aliases, and the HTTP files omit endpoint and authentication headers. Object and asset UUIDs came from the synthetic seed.

`cargo test -p xtask replay_pin8_riderless_crossing_both_ways` reads both HTTP exchanges and both viewer streams, checks the edits, destinations and link relationships, and verifies the six files against SHA256SUMS. A fresh capture can have different packet sequences, local IDs and timing.
