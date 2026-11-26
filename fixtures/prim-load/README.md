# Delivery of 500 persisted prims

Pin8 and Ferroan each delivered 500 distinct full ObjectUpdate blocks from the same committed SQLite file. At the 512,000 bit/s Task rate requested by both viewers, this run gave:

| Viewer | Distinct parts | Fresh repeat blocks | Blocks marked `RESENT` | First to last distinct part |
| --- | ---: | ---: | ---: | ---: |
| [Pin8](pin8.tsv) | 500 | 253 | 0 | 1,179 ms |
| [Ferroan](ferroan.tsv) | 500 | 0 | 0 | 801 ms |

Pin8 sent an identical second update for each of the 253 roots. Every child arrived once. The second updates had fresh packet sequences and a clear `RESENT` flag. The order changed between passes. For example, the roots ending in `0009` and `0006` first arrived in packets 21 and 22, respectively. Their repeats arrived in packets 104 and 100. The server and viewer captures agree on that order. The first viewer flag calculation after a store load can change the linkset's cached permission masks and queue the root again. Ferroan omits this identical root update ([TB138](../../docs/TRANQUILLITY_BUGS.md), [D101](../../docs/DECISIONS.md)).

Each receipt row identifies a synthetic store UUID. `first_ms` gives elapsed circuit time at first delivery. `first_resent` is the datagram's `RESENT` bit (0 or 1) on that first observation, even if the original packet was lost. `first_seq` and `first_fresh_repeat_seq` identify the packets carrying the first block and its first fresh repeat. A dash means no fresh repeat was observed. Subsequent identical blocks observed through logout are counted separately in `fresh_repeats` (bit clear) and `resent_repeats` (bit set). The delivery span is the largest first receipt minus the smallest. The Ferroan queue recorded zero new refusals and zero new drops.

A second run measured 1,181 ms on Pin8 and 800 ms on Ferroan at the same 512,000 bit/s Task rate. Both delivered 13 blocks per callback during the bulk transfer. Pin8 polls after 15 ms of idle time and allows a queue-empty callback at most once per 20 ms, so packet pairs arrived about every 30 ms. Ferroan wakes at the bucket deficit and `queue_empty_at` deadlines, giving about 20 ms between pairs. D101 accepts this timing difference, including the 18 ms on Pin8 and 9 ms on Ferroan to finish after restoring Task in Q65c3a.

Run `cargo xtask live run prim-load grid --physics basicphysics` on Linux under the shared live lock with a free port base and `UseSharedCompilation=false`. The Pin8 oracle persists the eight prim-capture parts and 246 more two-part linksets through `SQLiteSimulationData.StoreObject`. The store has 253 roots and 247 children, with 494 distinct stored texture entries. The added pairs have box or sphere roots and cylinder children with varied positions, relative offsets, rotations and scales.

The [store handoff](../store-handoff/README.md) commits a resize on C#, reopens the file for its reference viewer, waits for that C# home to exit, then opens the same file on Ferroan. The comparison checks the captured Task request and every block, including the committed scale. Before mapping the two circuit-local fields, each block must re-encode to the original bytes, and each parent must resolve to the root in the store. The run writes `grid/prim-load-Pin8.tsv` and `grid/prim-load-Ferroan.tsv` alongside the private captures.

Reference: Tranquillity `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`, `SceneObjectGroup.ProcessBackup`, `SQLiteSimulationData.StoreObject`, `LLClientView.ProcessEntityUpdates`, `LLUDPServer.cs` 1931 and `LLUDPClient.cs` 698–711.
