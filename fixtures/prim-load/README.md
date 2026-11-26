# Delivery of 500 persisted prims

Pin8 and Ferroan each delivered 500 distinct full ObjectUpdate blocks from the same committed SQLite file. At the 512,000 bit/s Task rate requested by both viewers, this run gave:

| Viewer | First deliveries | Repeated blocks | First to last distinct part |
| --- | ---: | ---: | ---: |
| [Pin8](pin8.tsv) | 500 | 253 | 1,179 ms |
| [Ferroan](ferroan.tsv) | 500 | 0 | 801 ms |

Each receipt file has one row per synthetic store UUID. `first_ms` is elapsed time on that viewer's circuit when it first received the part. `repeats` counts further full blocks for the same UUID observed through logout, including retransmissions. The elapsed delivery time is the largest first receipt minus the smallest. These figures describe one local run. The Ferroan entity-update queue recorded zero new refusals and zero new drops.

Run `cargo xtask live run prim-load grid --physics basicphysics` on Linux under the shared live lock with a free port base and `UseSharedCompilation=false`. The Pin8 oracle persists the eight prim-capture parts and 246 more two-part linksets through `SQLiteSimulationData.StoreObject`. The store has 253 roots and 247 children, with 494 distinct stored texture entries. The added pairs have box or sphere roots and cylinder children with varied positions, relative offsets, rotations and scales.

The [store handoff](../store-handoff/README.md) commits a resize on C#, reopens the file for its reference viewer, waits for that C# home to exit, then opens the same file on Ferroan. The comparison checks the captured Task request and every block, including the committed scale. Before mapping the two circuit-local fields, each block must re-encode to the original bytes, and each parent must resolve to the root in the store. The run writes `grid/prim-load-Pin8.tsv` and `grid/prim-load-Ferroan.tsv` alongside the private captures.

Reference: Tranquillity `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`, `SceneObjectGroup.ProcessBackup`, `SQLiteSimulationData.StoreObject` and `LLClientView.ProcessEntityUpdates`.
