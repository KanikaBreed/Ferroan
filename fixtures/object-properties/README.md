# Object properties, the physics event and two root kills

The estate owner and a second viewer logged in to Pin8 on the [Q65b stored-object scene](../prim-updates/README.md). The owner asked for family properties on every part, selected every part, read `ObjectPhysicsProperties`, then deleted the linkset root. Both viewers receive the two root kills. The published files keep the first two identical kill bodies. Each file is the message body after the id. Sequence numbers stay in the capture.

| File | What | Bytes |
| --- | --- | ---: |
| `family-1.bin` | `ObjectPropertiesFamily` for store part 1, the linkset root | 107 |
| `family-2.bin` | Root part 3 | 108 |
| `family-3.bin` | Root part 4 | 109 |
| `family-4.bin` | Root part 5 | 111 |
| `family-5.bin` | Root part 6 | 120 |
| `family-6.bin` | Root part 7 | 117 |
| `family-7.bin` | Root part 8 | 123 |
| `object-1.bin` | One `ObjectProperties` packet, eight blocks, store-uuid order | 1,512 |
| `kill-1.bin` | First `KillObject` for the linkset root | 5 |
| `kill-2.bin` | Second `KillObject` for the same root | 5 |
| `physics-1.xml` | One `ObjectPhysicsProperties` event, parts 1 through 8 | 2,027 |

The child of the linkset is part 2. Its family request is merged into the root, so the seven family files are the seven roots. Both kill bodies are the one-block packet for that root. The physics element lists the eight selected parts in the same store-uuid order. Empty integers use Encode2's `<integer />`. A zero real is `<real>0</real>`.

Before any alias is applied, the harness re-encodes each datagram and requires the captured bytes. The published body is the message after the id. Prim and owner UUIDs use the Q65b map. A kill local id and a physics `LocalID` use the alias of the part whose full update carried that live id. An avatar logout kill is left out.

Pin8 `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`. `RequestObjectPropertiesFamily` queues the root (`SceneObjectGroup.ServiceObjectPropertiesFamilyRequest`). `ObjectSelect` reaches `Scene.SelectPrim`, which sends `SendPropertiesToClient`. `DeRezAction.Delete` calls `DeleteGroupFromScene` (`SceneObjectGroup.cs` 2983–2986), which sends `KillObject` and then queues `PrimUpdateFlags.Kill`, so each open viewer receives two kill packets for the root. `ProcessEntityPropertyRequests` (`LLClientView.cs` 6222–6395) builds the family block (6396) and the object block (6431). Object-flag updates also enqueue `ObjectPhysicsProperties` (6370–6389).

The Pin8 delete took the trash path. `UseTrashOnDelete` defaults to true (`Scene.cs` 271), so `DeRezAction.Delete` goes to `takeDeleteGroups` and then `DeleteToInventory` (`Scene.Inventory.cs` 2962–2963, 3016–3020). That calls `DeleteGroupFromScene(false)`. After the `DeRezObject` the capture is `KillObject`, `PacketAck`, `KillObject`, `PacketAck`, then `UpdateCreateInventoryItem`. These eleven files leave that inventory packet out. `UpdateCreateInventoryItem` stays out until P2.8d2.

## What the capture proves

These bytes are the Q65b scene after the uuid alias in [map.json](../prim-updates/map.json):

- each family body is that request's root, in capture-uuid order, with request flags 0
- the object packet is eight blocks in store-uuid order, and each block's name and description are that part's
- creation dates are the stored unix time multiplied by 1,000,000, with the signed 32-bit product widened to 64 bits
- owner, group, creator and last owner are the stored ids after the alias. On this scene last owner equals owner
- the five masks and the sale type and price are the stored root words. The linkset child's stored masks and sale equal its root
- TouchName and SitName are empty, so each is one zero byte, and inventory serial is 0 because no part has task items
- both kill bodies are the one-block packet for the linkset root, local id 1 after the alias
- the physics element lists parts 1 through 8, with Encode2's empty integer and a zero real spelled `<real>0</real>`

Unit tests with distinct values cover a child's own last owner, name, touch name and sit name against the root's masks and sale, inventory serial 1 when the part has an item, a nonzero request-flags word, and a family request merged with a select in one queue slot.

## Reproduce

Build Pin8 `c401bceecd1684832ae8ba86a31b5ee01a4ded7b` with `cargo xtask live build`. Run `cargo xtask live run object-properties grid --physics basicphysics` under the shared live lock with a free port base and `UseSharedCompilation=false`. The home region reopens a copy of the scene with `SupportViewerObjectsCache=false`. The harness prints the family, object, kill and physics counts and the scratch directory. Copy the eleven bodies here.

## Ferroan compare

`cargo xtask live run object-properties grid --region ferroan` runs under the shared live lock with a free port base and `UseSharedCompilation=false`. The compare checks these eleven bodies and does not publish new files.
