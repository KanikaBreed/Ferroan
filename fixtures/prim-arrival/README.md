# Pin8 prim arrival on two viewer circuits

Two local test accounts received the same eight parts from the [Q65b SQLite scene](../prim-updates/README.md). Each viewer recorded its own full and terse datagrams, and the throttle requests it sent. Before redaction, the harness matched every recorded datagram byte for byte and by direction against the C# socket capture, including how many copies the server sent. Two circuits can receive identical bytes and sequence numbers.

| Viewer | Requested Task rate | Observed first deliveries |
| --- | --- | --- |
| [Control](control.cap) | 512,000 bit/s throughout | Parts 1–6 arrived together, then 7–8 arrived together 15 ms later. The original datagrams were 830 and 764 bytes. An avatar terse update followed before two further full packets repeated seven parts. |
| [Limited](limited.cap) | 8,000 bit/s, raised to 512,000 bit/s | Parts 1 and 2 arrived 256 ms apart. Six parts were outstanding when the viewer raised Task at 1,927 ms. It received them in six datagrams over the next 18 ms. |

The six other channel requests stayed byte-identical. Both groups fit the default Pin8 client ceiling. Times are elapsed milliseconds on each viewer's own monotonic clock. These are observations from one run. [arrival.txt](arrival.txt) preserves each packet boundary, flags, sequence, ordered blocks, repeats and avatar terse traffic. A part is shown as its Q65b alias, its region local ID and its parent local ID. Part 2 links to part 1 on both circuits.

## Reproduce

Run `cargo xtask live run prim-arrival grid --physics basicphysics` after the Pin8 live build. The home region reopens a copy of the scene with `SupportViewerObjectsCache=false`. The control account stays online and moves while the second account receives the same scene. The bot restores Task after observing partial delivery for at least 300 ms. Both viewers continue receiving traffic after the increase. The command reports the scratch output directory.

The reference is Tranquillity `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`. `ScenePresence.SendInitialData` requests the initial objects. `SceneObjectGroup.SendFullAnimUpdateToClient` queues the root followed by its linked parts. In `LLClientView.ProcessEntityUpdates` (4996–5431), C# budgets the dequeue, splits on the zero encoder's position and sends the full datagrams on Task. `LLUDPClient.SetThrottles` reads Task from bytes 16–19 of the request and clamps its rate to at least 1,000 bytes/s.

The capture files preserve directions, relative times, headers and block membership. The extractor aliases prim and owner UUIDs with the Q65b map, retains only identity and parent fields for full avatars, clears terse texture entries, and removes the session UUID and circuit code from throttle requests. Local IDs are the region's actual counters for this run. Replay with `cargo test -p xtask grid::prim_arrival`. One test compares the complete packet transcript and another moves a part across a datagram boundary while preserving the flattened part order.
