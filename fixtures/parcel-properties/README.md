# ParcelProperties elements on the multi-parcel store

One estate-owner circuit logged in to Pin8 on the [Q60b multi-parcel store](../parcel-overlay/README.md). The fixture is the original event-queue `<map>` for each requested moment. The poll id stays in the captured response and is not written here.

| File | Moment | Bytes |
| --- | --- | ---: |
| `arrival.bin` | Login completion on the remainder parcel, `Your Parcel`, sequence 0, single result | 3,738 |
| `request-cell.bin` | One land unit inside West (metres 5, 1, 6, 2), sequence 7, single result | 4,096 |
| `request-span-west.bin` | Span over West and East (metres 4, 0, 12, 4), sequence 8, multiple result, West | 4,105 |
| `request-span-east.bin` | The same span, sequence 8, multiple result, East | 4,032 |
| `walk.bin` | Walk east across the border at x=8, sequence 0, single result, East | 4,014 |

Pin8 `c401bceecd1684832ae8ba86a31b5ee01a4ded7b` sends `ParcelProperties` from `ScenePresence.CompleteMovement` (`ScenePresence.cs` 2475), from `LandManagementModule.ClientOnParcelPropertiesRequest` (1594) and from `EventManagerOnClientMovement` (536) when a walk changes parcel. A same-region teleport only moves the avatar (`EntityTransferModule.TeleportAgentWithinRegion` 703). The next `AgentUpdate` sends the landing parcel. That landing element is sequence 0 on West. The extractor checks it and does not publish it. The published walk element is the later sequence-0 East element.

The span covers land units (1, 0) and (2, 0). Pin8 scans x then y and sends West before East. Each file is one element. Snap selection is false. Empty integers use Encode2's `<integer />`. The bitmap is the original `<binary>` value.

Extraction slices each proxy HTTP 200 for `/CE/`. It reassembles every event in that body with the C# events header and the id trailer, and requires those bytes to equal the unredacted response before it copies a `ParcelProperties` element. `to_xml` of the parsed element does not reproduce the captured bytes.

## Reproduce

Build Pin8 `c401bceecd1684832ae8ba86a31b5ee01a4ded7b` with `cargo xtask live build`. Run `cargo xtask live run parcel-properties grid --physics basicphysics` under the shared live lock with a free port base and `UseSharedCompilation=false`. The harness prints:

```
parcel-properties: captured arrival, one cell, a two-parcel span and a border walk
parcel properties: arrival.bin 3738, request-cell.bin 4096, request-span-west.bin 4105, request-span-east.bin 4032, walk.bin 4014
```

To re-extract from the same raw run, use `cargo xtask live parcel-properties-fixtures <grid-run-directory>`. To install a new recording, copy its five files here and refresh the digest in `pin8_parcel_properties_replay_the_captured_elements` using that test's file order. Run `cargo test -p xtask parcel_properties`.
