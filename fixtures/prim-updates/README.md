# Pin8 full prim updates

[scene.db](scene.db) is a RegionStore 48 SQLite database for the harness home region. The regionstore oracle created the groups with `SceneObjectPart` and `SceneObjectGroup` and persisted them with `SQLiteSimulationData.StoreObject`. A separate C# region opened a copy with BasicPhysics and `[ClientStack.LindenUDP] SupportViewerObjectsCache=false`. One bot received full updates for all five prims. The sender is `Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs`, `CreatePrimUpdateBlock` (7572–7862), at Tranquillity `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`.

| Block | Stored part | Captured fields |
| --- | --- | --- |
| [part-1.bin](part-1.bin) | Box, root of the mesh linkset | Texture entry with a per-face material UUID, texture animation, UTF-8 text and color, media URL, particle system, sound and owner |
| [part-2.bin](part-2.bin) | Mesh child, offset (1, 2, 0.5) | Relative transform, sculpt and glTF material extra parameters, mesh profile begin 9375, hollow 27500 and path scale Y 150 |
| [part-3.bin](part-3.bin) | Tree, separate root | Vegetation layout and species byte 4 |
| [part-4.bin](part-4.bin) | Grass with a stored X rotation | Species 3 and zero rotation bytes |
| [part-5.bin](part-5.bin) | NewTree with the same stored X rotation | Species 3 and the normalized rotation |

Each binary contains one decompressed LLUDP `ObjectUpdate.ObjectData` block. Extraction checks the reliable and zero-coded flags and checks the decoded block against the original C# bytes before replacing any identifiers. It verifies that the captured prim UUIDs match the reopened store, that their local IDs are distinct and nonzero, and that the child's raw parent ID names the correct root. [map.json](map.json) maps each synthetic store UUID and its root to the redacted block. Local IDs and full UUIDs are numbered 1 through 5 in store UUID order. The parent ID follows the same mapping. The test account's nonzero owner UUID becomes UUID 100 (decimal), and a zero owner stays zero. Asset UUIDs, flags, transforms and payloads are preserved. The store has pseudo-CRCs 6501 through 6505 in that order. The C# blocks carry 6502, 6502, 6504, 6505 and 6506. On the first full update, `GenerateClientFlags` asks for the linkset's effective permissions. When they change, `SceneObjectGroup.AggregatePerms` calls `RootPart.ScheduleFullUpdate`, advancing only the root's CRC. Q65b added the two rotated vegetation cases. The original three C# blocks reproduced byte for byte.

The fixture uses BasicPhysics. The seed sets a sound radius of 12, and the wire carries 0. SQLite RegionStore has no sound-radius column. At Pin8, `SQLiteSimulationData.buildPrim` restores the looped sound UUID and gain and sets flags to 1 for a nonzero sound. The manifest records the synthetic store owner and its capture alias under `owner`.

## Reproduce

From the repository root on Linux, `cargo xtask regionstore prim-seed` rebuilds the seed at `.scratch/prim-seed/scene.db`. To refresh the store fixture, copy that file to `fixtures/prim-updates/scene.db`. The live run always opens a disposable copy of the fixture. Run the following under the shared live lock using the assigned port base and `UseSharedCompilation=false`:

```sh
cargo xtask live build
cargo xtask live run prim-capture grid --physics basicphysics
```

The run prints the directory containing the extracted blocks and UUID map. Compare its five binary files and `map.json` with this directory. The raw capture and live database stay under `.scratch/`. `cargo test -p xtask prim_capture` checks the committed blocks and their store relationships offline and exercises rejection of duplicate local IDs and incorrect parents.
