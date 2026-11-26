# Pin8 full prim updates

[scene.db](scene.db) is a RegionStore 48 SQLite database for the harness home region. The regionstore oracle created the groups with `SceneObjectPart` and `SceneObjectGroup` and persisted them with `SQLiteSimulationData.StoreObject`. A separate C# region opened a copy with BasicPhysics and `[ClientStack.LindenUDP] SupportViewerObjectsCache=false`. One bot received full updates for all eight prims. The sender is `Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs`, `CreatePrimUpdateBlock` (7572–7862), at Tranquillity `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`.

| Block | Stored part | Captured fields |
| --- | --- | --- |
| [part-1.bin](part-1.bin) | Box, root of the mesh linkset | Texture entry with a per-face material UUID, texture animation, UTF-8 text and color, media URL, particle system, sound and owner |
| [part-2.bin](part-2.bin) | Mesh child, offset (1, 2, 0.5) | Relative transform, sculpt and glTF material extra parameters, stored profile curve 0x30 sent as 0, mesh profile begin 9375, hollow 27500 and path scale Y 150 |
| [part-3.bin](part-3.bin) | Tree, separate root | Vegetation layout and species byte 4 |
| [part-4.bin](part-4.bin) | Grass with a stored X rotation | Species 3 and zero rotation bytes |
| [part-5.bin](part-5.bin) | NewTree with the same stored X rotation | Species 3 and the normalized rotation |
| [part-6.bin](part-6.bin) | Rotated glass root with a sit click action and particles only | Owner, signed twist, twist begin, radius offset, X/Y taper and skew, UTF-8 hover text, flexible, light, projected light image and reflection-probe parameters |
| [part-7.bin](part-7.bin) | Root with looping sound only | Owner, sound UUID, flags and gain, bounded hover text and media URL, stored Scripted flag cleared |
| [part-8.bin](part-8.bin) | Mesh root with AnimeshEnabled | Sculpt asset and extended mesh flags (bit 0 set) |

Each binary contains one decompressed LLUDP `ObjectUpdate.ObjectData` block. Extraction checks the reliable and zero-coded flags and checks the decoded block against the original C# bytes before replacing any identifiers. It verifies that the captured prim UUIDs match the reopened store, that their local IDs are distinct and nonzero, and that the child's raw parent ID names the correct root. [map.json](map.json) maps each synthetic store UUID and its root to the redacted block. Local IDs and full UUIDs are numbered 1 through 8 in store UUID order. The parent ID follows the same mapping. The test account's nonzero owner UUID becomes UUID 100 (decimal), and a zero owner stays zero. Asset UUIDs, flags, transforms and payloads are preserved. The store has pseudo-CRCs 6501 through 6508 in that order. The C# blocks carry 6502, 6502, 6504, 6505, 6506, 6507, 6508 and 6509. On the first full update, `GenerateClientFlags` asks for the linkset's effective permissions. When they change, `SceneObjectGroup.AggregatePerms` calls `RootPart.ScheduleFullUpdate`, advancing only the root's CRC. Q65b added the rotated vegetation roots, the particle-only and sound-only roots, and the animated mesh.

The LLUDP blocks cover all seven extra-parameter kinds. The emitter carries flexible (0x10), light (0x20), projected light image (0x40) and reflection probe (0x90). The mesh child carries sculpt (0x30) and glTF material IDs (0x80). The animated mesh root carries extended mesh flags (0x70).

After `StoreObject`, the oracle writes the speaker's hover text, media URL and Scripted flag directly into SQLite. The 255-byte text ends in `雪` after 126 copies of `é`. The final character crosses the 254-byte limit, so the wire carries 252 bytes plus NUL. The 257-byte URL ends in `🌐雪`; the globe fits exactly, so the wire carries 254 bytes plus NUL. The stored root has Scripted (0x40) set, and the reopened region clears it before sending the update.

The fixture uses BasicPhysics. The seed sets a sound radius of 12, and the wire carries 0. SQLite RegionStore has no sound-radius column. At Pin8, `SQLiteSimulationData.buildPrim` restores the looped sound UUID and gain and sets flags to 1 for a nonzero sound. The manifest records the synthetic store owner and its capture alias under `owner`.

The review ran each mutation against `stored_parts_match_every_byte_of_the_pin8_full_updates` alone. Changing the owner condition to require both sound and particles failed on the emitter. Omitting either owner case failed on the corresponding single-payload root. Clamping any negative path field to zero, losing twist begin or the ordinary root rotation, or restoring default material or click action failed on the emitter. Retaining Scripted or bypassing either text-field limit failed on the speaker. Changing any of the seven extra-parameter tags failed on the part that carries it.

## Reproduce

From the repository root on Linux, `cargo xtask regionstore prim-seed` rebuilds the seed at `.scratch/prim-seed/scene.db`. To refresh the store fixture, copy that file to `fixtures/prim-updates/scene.db`. The live run always opens a disposable copy of the fixture. Run the following under the shared live lock using the assigned port base and `UseSharedCompilation=false`:

```sh
cargo xtask live build
cargo xtask live run prim-capture grid --physics basicphysics
```

The run prints the directory containing the extracted blocks and UUID map. Compare its eight binary files and `map.json` with this directory. The raw capture and live database stay under `.scratch/`. `cargo test -p xtask prim_capture` checks the committed blocks and their store relationships offline and exercises rejection of duplicate local IDs and incorrect parents.
