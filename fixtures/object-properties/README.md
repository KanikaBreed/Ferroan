# Object properties, the physics event and two root kills

One estate-owner circuit logged in to Pin8 on the [Q65b stored-object scene](../prim-updates/README.md). The bot asked for family properties on every part, selected every part, read `ObjectPhysicsProperties`, then deleted the linkset root. Each file is the message body after the id. Sequence numbers stay in the capture.

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

Pin8 `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`. `RequestObjectPropertiesFamily` queues the root (`SceneObjectGroup.ServiceObjectPropertiesFamilyRequest`). `ObjectSelect` reaches `Scene.SelectPrim`, which sends `SendPropertiesToClient`. `DeRezAction.Delete` calls `DeleteGroupFromScene` (`SceneObjectGroup.cs` 2983–2986), which sends `KillObject` and then queues `PrimUpdateFlags.Kill`, so the viewer receives two kill packets for the root. `ProcessEntityPropertyRequests` (`LLClientView.cs` 6222–6395) builds the family block (6396) and the object block (6431). Object-flag updates also enqueue `ObjectPhysicsProperties` (6370–6389).

## Reproduce

Build Pin8 `c401bceecd1684832ae8ba86a31b5ee01a4ded7b` with `cargo xtask live build`. Run `cargo xtask live run object-properties grid --physics basicphysics` under the shared live lock with a free port base and `UseSharedCompilation=false`. The home region reopens a copy of the scene with `SupportViewerObjectsCache=false`. The harness prints the family, object, kill and physics counts and the scratch directory. Copy the eleven bodies here.

## Ferroan compare

`cargo xtask live run object-properties grid --region ferroan` runs under the shared live lock with a free port base and `UseSharedCompilation=false`. The compare checks these eleven bodies and does not publish new files.
