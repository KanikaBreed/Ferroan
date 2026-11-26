# Phase 2 gate map

Planning note, 26 November 2026. This file maps what is left before a person can log Firestorm into a Ferroan region on a grid where Robust and the neighbours are still C#, and do the Phase 2 scope in [PLAN.md](../PLAN.md). It proposes splits and one new row. It does not add those rows to [TASKS.md](../../TASKS.md) or [WORK_QUEUE.md](../../WORK_QUEUE.md). The manager assigns any new ids.

The C# citations are Pin8, `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`, read at that commit. Paths are relative to that tree. Where an older task cell cites `1d7abb8337` and the line has moved, this file uses the Pin8 line.

Standing decisions: [D38](../DECISIONS.md) (the walking stand-in, next door as teleport both ways plus child agents, viewer-side bakes, all three region stores) and [D39](../DECISIONS.md) (scene state on the frame thread, no held object crossing, parcels read and only the default parcel written, persistence timing, materials passed through). Phase 2's OAR load is terrain-height standing only. Full OAR and IAR round trips stay in Phase 5.

The queue still records two coders (rule 10). Section 4 is the parallel map for three, which is what this note was asked for.

## 1. The gate

[PLAN.md](../PLAN.md) states the Phase 2 scope: the LLUDP circuit, CAPS and the event queue, the scene, object and inventory updates, permissions, appearance and baking, teleports and crossings to and from C# regions, and loading OAR files with terrain-height standing only. After P2.0j, the region work is P2.0e2, P2.0e3, P2.1a through P2.12, then G3.

G3, in [TASKS.md](../../TASKS.md), is narrower than that sentence. It says the maintainer logs in with Firestorm and Aurora, walks, sees another avatar, builds, chats, and teleports to and from the C# region next door, following a Phase 2 section in [MANUAL_CHECKS.md](../MANUAL_CHECKS.md). That section does not exist yet. P2.12 writes it. [MANUAL_CHECKS.md](../MANUAL_CHECKS.md) says the Phase 2 gate is G3, after the later region work. G2, the first-login check, is still unrecorded. Later P2 rows do not wait on G2.

The table below is the person's script for the PLAN scope. Question 1 asks whether G3's shorter sentence replaces it. Automated evidence is a bot scenario, a capture compare, or a live run in [LIVE.md](../LIVE.md). [TESTING.md](../TESTING.md) keeps the harness as protocol evidence. A passing run is not the Firestorm result. The person records viewer versions in the G3 row. Ferroan has no physics switch: it always runs the walking stand-in. A C# walk, animation, or crossing capture that the compare uses is taken with `--physics basicphysics`, because the harness default is ubODE and the terse values differ.

`cargo xtask live run all` runs the names in the bot's scenario list. Today those pass lines include `login: pass`, `walk: pass (forward movement, observer kills=1, leaving viewer kills=0)`, `chat: pass`, `build: pass`, `teleport: pass` with the landing position, `texture: pass`, and `inventory: pass`. `cargo xtask live compare region` prints `region: match` when both a C# home and a Ferroan home print `login: pass`, `chat: pass`, `teleport: pass`, `teleport neighbour: pass (1001,1000 and back)`, and `live: run passed`. Dedicated probes that `all` leaves out, and that later rows add, are named in the table. P2.12 asks the Ferroan `all` run to print the same lines as C# for walk, avatars, build, link, take, rez, store swap, and object cross, plus teleport both ways and the neighbour in view.

| What the person does | Automated evidence that must already pass |
| --- | --- |
| Log in through C# Robust and arrive standing on the Ferroan region | Already passing: `cargo xtask live run login grid --region ferroan` prints `login: pass`, and `compare region` includes it. Circuit, seed, event queue, arrival, and logout are P2.0a through P2.0d4 and P2.0d6. `cargo xtask live run caps grid --region ferroan` covers the seed and the held empty poll. |
| See the region and its terrain, standing at terrain height | Already passing: P2.0e `LayerData` matches the stored heightmap and `AgentMovementComplete` puts the avatar at that height. P2.0e2 and P2.0e3 write and load the default terrain on SQLite, MySQL, and PostgreSQL. The walk line lands at that height. |
| See objects stored in the region | Full updates of a handed-off store already pass: `cargo xtask live run store-handoff grid --physics basicphysics` and `cargo xtask live run prim-load grid --physics basicphysics` (P2.5a4a and P2.5a4b). Before this Firestorm check, P2.5b must match compressed and cached updates, with the bot's handshake flag bit 2 clear, and P2.5c must match interest-list enter and leave plus `ObjectProperties`. Firestorm asks for the cached path. The current bot handshake sends `0x7`, which turns those probes off, so prim-load does not yet prove the path Firestorm uses. |
| Walk | `walk: pass` already, from the stand-in (P2.2b1, P2.2b2). Before the person walks, P2.3b1's state tests and P2.3b2's observer must show walk then stand with the same animation UUIDs and sequence numbers as a basicphysics C# region. The existing `walk: pass` line does not check those UUIDs. |
| See another avatar | Already passing: `avatars: pass` (P2.3a) and `UUIDNameReply` (P2.3d). Before the person looks, P2.3c2's observer must receive the same `AvatarAppearance` texture entry C# sends, and P2.3b2's animation packets must match. Without P2.3c2 the other avatar stays grey or Ruth. |
| Chat | Already passing: `chat: pass` (P2.0g), including the second account on channel 0 and DEBUG_CHANNEL type 6. P2.0g is DONE. The Ferrobot table in [LIVE.md](../LIVE.md) still lists Ferroan local chat as pending. |
| Edit and rez an object | P2.8a through P2.8c must pass first: rez, select, move, rotate, texture, link, and unlink, with `build: pass` on the Ferroan region and a new `link: pass`. Rez from inventory is P2.8e. Permissions for rez, edit, move, delete, take, link, and copy are P2.7a through P2.7c, already DONE, and these rows call them. |
| Use inventory | The window and folder fetch already pass: `inventory: pass` and the fetch caps (P2.0d7). Take into inventory and delete to trash are P2.8d (`take: pass`, asset XML compared with C#). Rez of that item is P2.8e, both directions. |
| Change appearance | P2.0f already sends the unbaked `AvatarAppearance` and avatar `ObjectUpdate`. P2.3c1 must match wearables, cached-texture, and `UploadBakedTexture` replies with C# for the same account. P2.3c2 must store through `setavatar` and broadcast the baked texture entry. `ServerSideBakingModule` stays out (D38). |
| Teleport to a C# region and back | Already passing: `teleport neighbour: pass (1001,1000 and back)` and `cargo xtask live run hop grid --region ferroan` / `neighbour grid --region ferroan` (P2.0i). |
| Cross a border | Read as D38's next-door pair plus an object crossing. The neighbour is in view while the avatar stays root at home: P2.11a's `cargo xtask live run child-view grid --physics basicphysics` with Ferroan as home, then P2.11b the other way, then P2.11c's throttle fields on those same messages. An object crosses both ways with `SupportsHeldCrossing` false: P2.10b and P2.10c, `object cross: pass`, against the riderless P2.10a capture. Walking across the border stays deferred (D38). Question 2 is this reading. |
| Stand on an OAR-loaded region | No P2 row covers this. Section 3 proposes one. The load uses the `no-objects` option. The avatar stands at that terrain height, and `LayerData` matches a C# region that loaded the same archive. Objects, assets, parcels, and region settings in the archive stay out. Those are Phase 5. |

P2.12 is the report that gathers those lines, adds the Phase 2 section to [MANUAL_CHECKS.md](../MANUAL_CHECKS.md), and stops. G3 is the person, after that section exists. A Ferrobot session that watches another avatar walk and build is part of P2.12's evidence, as far as the pinned Ferrobot API allows. Region IM stays out. The Ferrobot IM line is still pending and is not a Phase 2 gate check.

## 2. Rows that are not DONE

Thirty-one P2 rows are not `DONE`. Each note says what the row still needs, what it depends on, whether the gate waits for it, and whether it is more than about a day. A proposed split is a proposal. The parent stays open until every part is `DONE`. The manager assigns the queue ids.

### Bookkeeping, no remaining code

These parents are still `BLOCKED` because they were split. Every leaf under them is `DONE`. Closing the parent is a status change. It is on the critical path because queue rule 1 will not start an item whose `after` row is still `BLOCKED`, and because P2.12's dependency text is the range P2.1a through P2.11c except P2.5d.

| Row | Queue | What is left | Depends on | Critical path |
| --- | --- | --- | --- | --- |
| P2.5a | Q65, still `BLOCKED` (split into Q65a–Q65d) | Nothing to code. Leaves P2.5a1, P2.5a2, P2.5a3a, P2.5a3b, P2.5a3c1, P2.5a3c2, P2.5a4a, and P2.5a4b are `DONE`, including the store handoff (#304) and the 500-part load. | P2.1b2, P2.4a1, both `DONE` | Yes, as a status close. Q66 and Q67 are `READY after Q65` and do not start while Q65 stays `BLOCKED`. |
| P2.5a3 | Q65c, still `BLOCKED` | Nothing to code. Q65c1, Q65c2, Q65c3a, and Q65c3b are `DONE`. | Those leaves | Yes, the same status close. |
| P2.5a3c | Q65c3, still `BLOCKED` | Nothing to code. The mixed Task drain and the material-override datagram are `DONE`. The `RenderMaterials` cap stays in P2.5d. | P2.5a3b, `DONE` | Yes, the same status close. |
| P2.5a4 | Q65d, still `BLOCKED` | Nothing to code. Q65d1 and Q65d2 are `DONE`. | P2.5a3c1, `DONE` | Yes, the same status close. |
| P2.6a | Q60, still `BLOCKED` (split into Q60a–Q60c) | No overlay code left. Q60a, Q60b, Q60c1, Q60c2, and Q60c3 are `DONE`. The one open child is P2.6a3a2, below, and it is a separate row. | P2.0e, `DONE` | Yes, as a status close, because P2.9a's task dependency lists P2.6a. Q74 itself says `READY after Q70` only. Rule 1 also requires the task dependencies. |
| P2.6a3 | Q60c, still `BLOCKED` (manager to review the Q60c split) | No delivery code left. Q60c3 delivered the overlay on the Land throttle. | P2.6a2, `DONE` | Yes, as a status close. P2.6b's task dependency lists P2.6a3. Q61c says `READY after Q61a, Q61b and Q60c3`, and Q60c3 is `DONE`. |

P2.0d5 is a stale status, not remaining work. The cell is `TODO`. Its own evidence is [PR #90](https://github.com/KanikaBreed/Ferroan/pull/90), implementation `1674cf6`, with `cargo xtask http status-verify` in the live job. Q10h is `DONE (#90)`. It depends on P2.0d3, which is `DONE`. It is outside P2.12's dependency range. Critical path: no. The manager can mark the cell `DONE` from the evidence already in the row.

### Off the gate

**P2.5d.** `TODO`. Not queued. The `RenderMaterials` cap and the `needMaterials` path (`Source/OpenSim.Region.OptionalModules/Materials/MaterialsModule.cs`, 1458 lines at the older pin; the row still says a few days, as a guess). Depends on P2.5a. D39 keeps Phase 2 to material ids in the texture entry and extra params. The negotiated UDP material send is already in P2.5a3c2. Critical path: no. Leave it unsplit until someone queues it after the gate.

### Already split

These stay as the approved notes. This file does not split them again.

**P2.6b / Q61.** `TODO`. Parcel properties over the event queue on arrival, on `ParcelPropertiesRequest` (`LLClientView.cs` 10874, `SendLandProperties` 6757), and when the parcel changes while walking. Parcel editing stays in Phase 5 (D39). Depends on P2.6a3 and P2.2b2. Q61a is `DONE` and published the capture. Q61b (`READY after Q61a`) encodes `SendLandProperties` and matches each captured element, about a day. Q61c (`READY after Q61a, Q61b and Q60c3`) delivers the three paths and compares live with Pin8, about a day. Q60c3 is `DONE`, so Q61c waits on Q61b. Critical path: yes. P2.12's range includes P2.6b. The person's script in section 1 does not name the About Land dialog. The row is still in the written gate dependency.

**P2.11a / Q79.** `BLOCKED`. Ferroan opens a child on the C# neighbour so that neighbour is in view (D38). Depends on P2.0i and P2.2b2, both `DONE`. Q79a, Q79b, and Q79c1 are `DONE`. There is no `P2.11a3b` row on this branch. The approved Q79c2 note names the two parts that finish the parent:

1. **Q79c2a (P2.11a3b1),** `READY after Q79c1`, about a day. Finish every child claim once, and close children on a far hop. No frame loop and no live grid. Pin8 `CloseChildAgents` is `ScenePresence.cs` 5060–5071. `InformClientOfNeighbourAsync` sleeps 500 ms after a successful `CreateAgent` and before `EnableSimulator` (`EntityTransferModule.cs` 2780). `GetChildAgentsToClose` then `CloseChildAgents` run at `EntityTransferModule.cs` 1280 (V1) and 1474 (V2).
2. **Q79c2b (P2.11a3b2),** `READY after Q79c2a`, about a day. Drive the neighbour set from the frame. Arrival calls `EnableChildAgents` at `ScenePresence.cs` 2575 and 4398. The child half of `CheckForSignificantMovement` is 4750–4828. Evidence is `cargo xtask live run child-view grid --physics basicphysics` with Ferroan as home, against the Q79a capture.

P2.11a stays `BLOCKED` until both are `DONE`. Critical path: yes. Walking across the border stays deferred.

### Rows that need a split

Each proposal is more than about a day at Pin8 `c401bceecd`. The parent stays `TODO` until every part is `DONE`.

**P2.3b1.** `TODO`. The animation state machine. Depends on P2.2b2 and P2.3a, both `DONE`. Q49 is `READY after Q46b and Q48`, and both are `DONE`, so this can start now. Critical path: yes. The person's walk and the other avatar's motion use it. `ScenePresenceAnimator.cs` is 885 lines, `AnimationSet.cs` is 361, and `DefaultAvatarAnimations.cs` is 124, the same sizes the row names.

1. **Movement states** (about one day). `DetermineMovementAnimation` (`ScenePresenceAnimator.cs` 260–570), `UpdateMovementAnimations` (576–594), and `TrySetMovementAnimation` (187). The stand-in supplies "on ground" in place of `IsColliding`. Unit tests for walk, run, fly, fall, land, and stand, matching the states a basicphysics C# region reaches.
2. **Worn and overridden animations** (about one day, after part 1). `AddAnimation` (81 and 100), `RemoveAnimation` (122 and 156), `ResetAnimations` (168), and `AnimationSet.cs`. With nothing worn, the movement state from part 1 still wins.

`GenerateRandomAnimation` (625) is outside the six transitions the row names. `SendAnimPack` (823, 838, and 854) is P2.3b2.

**P2.3c1.** `TODO`. Viewer-side bakes, caps and circuit only. Depends on P2.0f, `DONE`. Q51 is `READY after Q18a`, which is `DONE`, so this can start now. Critical path: yes. `AvatarFactoryModule.cs` is 1518 lines. `UploadBakedTextureModule.cs` is 225.

1. **Wearables** (about one day). `AgentWearablesRequest` / `AgentWearablesUpdate` through `Client_OnRequestWearables` (`AvatarFactoryModule.cs` 1275). Replies compared with C# for the same account.
2. **Cached textures** (about one day, after part 1). `Client_OnCachedTextureRequest` (1437) and `ValidateBakedTextureCache` (549). The bot's cached-texture query matches C#.
3. **UploadBakedTexture** (about one day, after part 1; it can follow part 2 or run beside it once the wearables reply exists). `UploadBakedTextureModule.cs` and `SaveBakedTextures` (315). The uploaded bake matches C#.

The row's guess stands: confirm which bake path this grid's Firestorm OpenSim build uses while doing part 1. `ServerSideBakingModule` stays out.

**P2.3c2.** `TODO`. `AgentSetAppearance`, store, and broadcast. Depends on P2.3c1 and P2.3a. Q52 waits on Q51 and Q48. Q48 is `DONE`. Critical path: yes. Without it, Firestorm users see each other grey or as Ruth.

1. **Accept and store** (about one day, after P2.3c1). `Client_OnSetAppearance` (1296), `SetAppearance` (195, and the overloads at 201 and 221), `SaveAppearance` (884), and `SetAppearanceAssets` (940). The avatar row matches C#'s `setavatar` result. Bot command: `AgentSetAppearance`.
2. **Broadcast** (about one day, after part 1). `SendAppearance` (274). The observer receives the same `AvatarAppearance` texture entry C# sends.

**P2.5b.** `TODO`. Compressed and cached updates. Depends on P2.5a. Q66 is `READY after Q65`, so it waits on the P2.5a status close, then it can run beside P2.5c. P2.8a depends on P2.5c, so this row does not lengthen the object spine. Critical path: yes for the Firestorm "see stored objects" check, because `SupportViewerObjectsCache` defaults to true and the current bot turns the probes off.

1. **Compressed blocks** (about one day). `CreateCompressedUpdateBlock` (`LLClientView.cs` 7892–8133) and `CreateCompressedUpdateBlockZC` (8134). Bytes compared with a C# capture of the same stored objects.
2. **Cached probes** (about one day, beside part 1 once the compressed block can be hashed). `ObjectUpdateCached` header at 4989, the cached copy at 5532, and `HandleRequestMultipleObjects` at 9744 (registered at 8580). A bot mode with handshake flag bit 2 clear. The exact Firestorm flag value comes from a capture. The bot sends `0x7` today.

**P2.8c.** `TODO`. Link, unlink, and duplicate. Depends on P2.8b2. Q71 is `READY after Q70`. Critical path: yes. P2.12 names `link: pass`.

1. **Link and unlink** (about one day). `HandleObjectLink` (`LLClientView.cs` 9624), `Scene.Inventory.cs` `LinkObjects` (3679 and 3690), `SceneGraph.cs` `LinkObjects` (1940), `SceneObjectGroup.cs` `LinkToGroup` (4199), and `DelinkFromGroup` (4489, with the other overloads at 4445, 4462, and 4614). `link: pass` for a two-prim link and an unlink. The prims-per-linkset limit applies.
2. **Duplicate** (about one day, after part 1). `SceneGraph.cs` `DuplicateObject` (2188), `Scene.cs` `DuplicateObject` (3778), and `HandleObjectDuplicate` (`LLClientView.cs` 9726). The observer sees the copy.

**P2.8d.** `TODO`. Take and delete to trash. Depends on P2.8a and P2.S1. P2.S1 is `DONE`. Q72 is `READY after Q68 and Q57`, and Q57 is `DONE`, so take can start when rez-and-select lands. It does not wait for texture edit. Critical path: yes.

1. **Delete** (about one day, after P2.8a). `DeRezObjects` (`Scene.Inventory.cs` 2870) for `DeRezAction.Delete` (2958). That branch either trash-takes or hard-deletes, from `m_useTrashOnDelete` at 2962. The other viewer gets `KillObject` (P2.5c), and the prim is gone from the store.
2. **Take and TakeCopy** (about one day, after part 1). `DeRezAction.Take` (2944) and `TakeCopy` (2935) through `InventoryAccessModule.cs` `CopyToInventory` (442). The asset XML matches what C# writes, and the new item is in inventory. `take: pass`.

`DeRezAction.Return` (2970) and `GodTakeCopy` (2951) are question 6. The two parts above cover the actions the row names.

**P2.8e.** `TODO`. Rez from inventory. Depends on P2.8d and P2.S2. P2.S2 is `DONE`. Q73 waits on Q72 and Q58. Q58 is `DONE`. Critical path: yes. P2.12 names rez.

1. **One user-inventory object** (about one day, after P2.8d). `Scene.Inventory.cs` `RezObject` (3269). The `fromTaskID` zero path (3278–3285) calls `InventoryAccessModule.RezObject` (892, and the overloads at 902, 933, and 944). The ray is placed on terrain (D38). A no-copy item is removed. Rez an item the C# region took, and the reverse, for a single object.
2. **Coalesced rez** (about one day, after part 1). `GetObjectsToRez` (`Scene.Inventory.cs` 3138) and the next-owner masks. P2.S2 and P2.S3 already cover the XML reader and the coalesced bytes.

Rez from a prim's inventory (`fromTaskID` non-zero, 3288) is question 7.

**P2.9a.** `TODO`. SQLite persistence. Depends on P2.8b2 and P2.6a. Q74 is `READY after Q70`. Rule 1 also waits until P2.6a is `DONE`. Ferroan and C# never open the same store at once (D39). Critical path: yes. This is the second half of the store swap, and D38 puts every region store on the gate through P2.9b.

1. **One prim, forced** (about one day, after P2.8b2 and the P2.6a status close). `SQLiteSimulationData.cs` `StoreObject` (534), `RemoveObject` (557), `fillPrimRow` (2132; the next method, `fillShapeRow`, is 2549), `fillShapeRow` (2549), and `StorePrimInventory` (2632). `ForceSceneObjectBackup` (`Scene.cs` 2235) calls `ProcessBackup` (`SceneObjectGroup.cs` 3162). A Ferroan stop and start on the same SQLite file keeps the prim. `fillPrimRow` is a long column writer. If the oracle dump overruns the day, split that writer off on its own and keep the forced-backup check with it.
2. **Periodic backup and the default parcel** (about one day, after part 1). `Backup` (`Scene.cs` 2170), `UpdateStorageBackup` (2142), the `m_dontPersistBefore` / `m_persistAfter` window (318, 320, read at 1071–1074), and `Backup(true)` on shutdown (1615). `StoreLandObject` (1018) writes only the default whole-region parcel. Evidence is the second half of the store swap: build on Ferroan, stop, start C# on the same file, and read the same objects.

**P2.9b.** `TODO`. MySQL and PostgreSQL persistence, the same rules as P2.9a, plus `cargo xtask regionstore cross` for writes. Depends on P2.9a and P2.4b. P2.4b is `DONE`. Q75 waits on Q74 and Q55. Q55 is `DONE`. Critical path: yes, because D38 requires all three databases at the gate. A SQLite-only Firestorm session can be rehearsed earlier. The gate itself waits for this row.

1. **MySQL** (about one day, after P2.9a). `MySQLSimulationData.cs` `StoreObject` (130) and `RemoveObject` (260). The store swap on MySQL, and the MySQL half of `regionstore cross` for writes.
2. **PostgreSQL** (about one day, after part 1, or beside it once part 1 has proved the writer). `PGSQLSimulationData.cs` `StoreObject` (259) and `RemoveObject` (448). The store swap on PostgreSQL, and the PostgreSQL half of the same cross.

**P2.11b.** `TODO`. Ferroan accepts a child `CreateAgent` from the C# neighbour, which it refuses today, and an `AgentPosition` PUT that checks the session before it changes state. Depends on P2.11a. Q80 is `READY after Q79c2b`. Critical path: yes (D38, both directions). The throttle scale inside the position update is P2.11c, not these parts.

1. **Accept the child** (about one day, after Q79c2b). The child branch of `NewUserConnection` (`Scene.cs` 4117). The child-versus-root log is at 4137. An existing child is reused at 4211. The circuit stays a child. Evidence: a C# neighbour's child POST is accepted, and the bot does not become root.
2. **Position and the full child update** (about one day, after part 1). `IncomingUpdateChildAgent(AgentPosition)` (`Scene.cs` 4848) refuses a session mismatch at 4860 and calls `ScenePresence.UpdateChildAgent(AgentPosition)` (5141). `IncomingUpdateChildAgent(AgentData)` (4773) refuses a session mismatch at 4819 and calls `UpdateChildAgent(AgentData)` (5125). Evidence: the child pose matches the capture, and a wrong session is refused. Part 2 may apply throttles at factor 1. P2.11c replaces that with the distance scale.

### About a day, leave whole

**P2.3b2.** `TODO`. Animation packets and viewer-started animations. Depends on P2.3b1. Q50 waits on Q49. Critical path: yes. At this pin the packet send is `LLClientView.SendAnimations` (4666), fed by `ScenePresence.SendAnimPack` (4642) and `ScenePresenceAnimator.SendAnimPackToClient` (838). `HandleAgentAnimation` is `LLClientView.cs` 9466. The older cell's `ScenePresence.SendAnimations` (4666) is this `LLClientView` method. Evidence: the observer sees walk, then stand, with the same UUIDs and sequence numbers as C# running basicphysics. About a day. No split.

**P2.5c.** `TODO`. Interest list and properties. Depends on P2.5a. Q67 is `READY after Q65`. Critical path: yes. P2.8a waits on it. Draw-distance culling, `SendKillObject` (`LLClientView.cs` 2374), `SendObjectPropertiesFamilyData` (6173), and `SendObjectPropertiesReply` (6203). Evidence: the observer sees objects enter and leave view as on C#, and property bytes match. About a day, as the row says. No split.

**P2.6a3a2.** `TODO`. Read a viewer's group memberships from the Local SQLite `os_groups` connector. Depends on P2.6a3a, `DONE`. Q86 is `READY after Q60c1`, which is `DONE`, so this can start now. Pin8's type is `GroupsServiceLocalConnectorModule` in `Addons/OpenSim.Addons.Groups/Local/GroupsServiceLocalConnectorModule.cs` (class at 41). `GetAgentGroupMemberships` is at 298 and forwards to the groups service. Q60c1 refuses that connector at startup until this row lands. The harness grid uses Robust, and Q60c1 already reads remote memberships. Evidence: member, non-member, and owner cases against Pin8 on the same SQLite store. About a day. No split. Critical path: no for a Robust-grid Firestorm login. It sits inside P2.12's written range because that range is P2.1a through P2.11c. Question 4.

**P2.8a.** `TODO`. Rez a new prim and select it. Depends on P2.5c and P2.7c. P2.7c is `DONE`. Q68 waits on Q67 and Q64. Q64 is `DONE`. Critical path: yes. `HandleObjectAdd` (`LLClientView.cs` 9659), `Scene.AddNewPrim` (`Scene.cs` 2682), `SelectPrim` and `DeselectPrim` (`Scene.PacketHandlers.cs` 196 and 272), `HandleObjectName` (10012), and `HandleObjectDescription` (10001). The ray is placed on terrain (D38). Evidence: the first half of `build: pass` on the Ferroan region, and the observer sees the new prim. About a day. No split.

**P2.8b1.** `TODO`. Edit, transform. Depends on P2.8a. Q69 waits on Q68. Critical path: yes. P2.10b also waits on it. `HandleMultipleObjUpdate` (`LLClientView.cs` 12623), `UpdatePrimScale` (`SceneGraph.cs` 1545), `UpdatePrimGroupRotation` (1645 and 1664), and `UpdatePrimGroupPosition` (1700–1724). Evidence: the move half of `build: pass`, plus a rotate check, with the observer's updates compared with C#. About a day. No split.

**P2.8b2.** `TODO`. Edit, appearance and flags. Depends on P2.8b1. Q70 waits on Q69. Critical path: yes. Link, persistence, and the rest of `build: pass` wait on it. The five handlers at this pin are short enough to stay one row: `HandleObjectShape` (`LLClientView.cs` 9674), `HandleObjectExtraParams` (9711), `HandleObjectFlagUpdate` (9818; the physical flag is stored and does nothing until Phase 3), `HandleObjectImage` (9861; the next handler starts at 9891), and `HandleObjectPermissions` (10023) through `Scene.HandleObjectPermissionsUpdate` (`Scene.cs` 5389). Evidence: all of `build: pass`, plus a texture check. About a day. No split.

**P2.10a2.** `TODO`. The seated-avatar object-crossing fixture, two Pin8 C# regions only. Depends on P2.10a, `DONE`. Q76b is `READY after Q76`, which is `DONE`, so this can start now. It records the rider's agent transfer beside the object post, and what each viewer receives. The riderless path is already captured. Pin8's access check at `EntityTransferModule.cs` 1764 uses the agent's own UUID. A teleport while sitting stands the avatar up at `EntityTransferModule.cs` 1001 before the transfer. Held crossing stays off (D39). About a day of capture. No split. Critical path: no for the Ferroan object-cross code. P2.10b does not depend on it. It sits inside P2.12's written range. Question 5.

**P2.10b.** `TODO`. Object crossing, Ferroan to C#. Depends on P2.10a, P2.S1, P2.8b1, and P2.0i. The first, third-listed as a task, and the last of those are `DONE` except P2.8b1. Q77 waits on Q76, Q57, Q69, and Q18c. Critical path: yes. The `AbsolutePosition` setter (`SceneObjectGroup.cs` 883) calls `StartCrossing` (900 and 960) when the new position is outside the region. `CrossPrimGroupIntoNewRegion` (`EntityTransferModule.cs` 2968) posts through `SimulationServiceConnector.CreateObject` (487) and then deletes the local object (2988). `state` passes through unchanged (D39). Evidence: the region-to-region bodies match the riderless P2.10a capture, and the C# region shows the object. About a day. No split.

**P2.10c.** `TODO`. Object crossing, C# to Ferroan. Depends on P2.10b and P2.S2. Q78 waits on Q77 and Q58, and does not wait on P2.9a. Critical path: yes. `ObjectSimpleHandler.DoObjectPost` (`Source/OpenSim.Server.Handlers/Simulation/ObjectHandlers.cs` 102; the file is 265 lines) accepts the POST. `Scene.IncomingCreateObject` (3083–3124) places the object. Persist once P2.9a exists, as the row already says. `SupportsHeldCrossing` stays false, so `Scene.CrossingHold.cs` stays off. Evidence: the reverse of P2.10b against the same capture, and `object cross: pass` both ways. About a day. No split.

**P2.11c.** `TODO`. Child-agent throttles. Depends on P2.11b, P2.1c, and R26. P2.1c and R26 are `DONE`. Q85 waits on Q80 and QR26. Critical path: yes. It is the scale on the messages section 1's border check already compares. A child's `UpdateChildAgent(AgentPosition)` (`ScenePresence.cs` 5141–5187) scales throttles by `1 - 0.3 * (x² + y²) / 256²`, floored at 0.2, through `LLClientView.SetChildAgentThrottle` (12853) and `LLUDPClient.SetThrottles` (370). An arriving `AgentData` applies throttles at factor 1 (`CopyFrom`, 5332–5333). Outgoing child updates carry `GetThrottlesPacked(1)` (`ScenePresence.cs` 4819 and 5214, `LLUDPClient.cs` 460). R26 owns the rest of the outgoing `AgentData` fields. About half a day. No split.

**P2.12.** `TODO`. The gate report. Depends on P2.1a through P2.11c except P2.5d, plus P2.0e2, P2.0e3, and P2.3d. Q81 is `READY after Q40 to Q80 and Q82 to Q85`. Writing the checklist and the [MANUAL_CHECKS.md](../MANUAL_CHECKS.md) section is about a day once those rows are `DONE`. It cannot start earlier. Critical path: yes. It is the last row before G3. Several queue ids inside "Q40 to Q80" are parents whose leaves are already `DONE` and whose own status is still `SPLIT` or `BLOCKED`: Q40, Q46, Q53, Q55, Q60, Q60c, Q65, Q65c, Q65c3, and Q65d. Rule 1, read literally, keeps Q81 waiting on those statuses too. Question 9.

## 3. Gaps

One gate check has no P2 row.

**Suggested row P2.0e4, OAR load, terrain-height standing only.** The manager assigns the id and the queue row. PLAN's Phase 2 sentence includes loading OAR files with terrain-height standing only. No current P2 row names an archive, `ArchiveReadRequest`, or `load oar`. Phase 5 keeps full OAR and IAR round trips, including objects, assets, parcels, and region settings.

Pin8 loads an archive from `ArchiverModule.HandleLoadOarConsoleCommand` (`Source/OpenSim.Region.CoreModules/World/Archiver/ArchiverModule.cs` 95). The `no-objects` option is registered at 127 and stored at 227. `ArchiveReadRequest` (`ArchiveReadRequest.cs` 234) sets `m_noObjects` from that option. Object paths are skipped when the flag is set (475–477). Terrain paths call `LoadTerrain` (498–501). `LoadTerrain` (1271) asks the terrain module to `LoadFromStream` (1284 for a plain load). Settings (503) and land data (508) are separate branches and stay out of this row.

Depends on P2.0e, which is `DONE`. The heightmap writer from P2.0e2 is the store side. This row is the console load. About one day: one console path, the `no-objects` flag, and a terrain stream. No split.

Evidence: load an archive with `no-objects` on Ferroan and on a C# region. `LayerData` matches. The avatar stands at that height under the walking stand-in. An object entry in the archive does not appear. Assets, parcels, and region settings in the archive are unchanged. Critical path: yes under the PLAN sentence. Under G3's current one-line list, the person never loads an archive. Question 1 decides which reading the gate uses.

Considered, and not proposed as rows:

- Permissions. P2.7a, P2.7b, and P2.7c are `DONE`. Edit, take, and delete call them.
- Wind and cloud `LayerData`. Terrain `LayerData` is P2.0e. The gate does not ask for wind or cloud.
- Worn attachments. P2.0f sent no attachment block. `Client_OnAvatarNowWearing` is `AvatarFactoryModule.cs` 1311 and `MergeNowWearing` is 1370. Question 8 asks whether the appearance check includes a worn attachment. Attachment rez stays out of Phase 2 either way.
- The draw-distance ini values. P2.5c is the interest list. Stored objects already arrive as full updates through the done P2.5a leaves. A separate ini follow-up is not required to name a new row.
- Mesh upload that builds a scene object. It is not one of the checks in section 1.
- Walking across the border. D38 defers it. Section 1's border check is the child-agent view plus the object crossing.
- `RenderMaterials`. P2.5d already exists and D39 leaves it past the gate.

## 4. Critical path and three coders

The long pole is the object spine, and it is stuck on a status close. Q66 and Q67 are `READY after Q65`. Q65 is `BLOCKED` even though every leaf is `DONE`. Until the manager marks Q65, Q65c, Q65c3, and Q65d `DONE`, nobody starts compressed updates, the interest list, or anything that follows them (rez, edit, link, persistence, object cross out).

Dependency order of the rows the gate waits for:

1. Status close: P2.5a, P2.5a3, P2.5a3c, P2.5a4, and the matching queue parents. Also P2.6a and P2.6a3, so P2.9a and P2.6b satisfy rule 1. P2.0d5's cell can be corrected in the same pass. It does not unblock the spine.
2. In parallel once those parents are `DONE`, or immediately where the queue is already `READY`:
   - Object spine: P2.5c, then P2.8a, then P2.8b1, then P2.8b2. From P2.8b2, link (P2.8c) and SQLite persistence (P2.9a, then P2.9b). From P2.8a, take (P2.8d, then P2.8e). From P2.8b1, object cross out (P2.10b, then P2.10c). P2.5b runs beside P2.5c and does not gate the spine.
   - Appearance: P2.3c1, then P2.3c2. Animations: P2.3b1, then P2.3b2. Both chains can start today.
   - Children: Q79c2a, then Q79c2b, which finishes P2.11a, then P2.11b, then P2.11c. Q79c2a can start today.
   - Parcels: Q61b, then Q61c. Q61b can start today.
   - OAR terrain, the suggested P2.0e4, any time. It depends only on P2.0e.
3. P2.12, after the range it names is `DONE` and the queue parents in "Q40 to Q80" are `DONE` if question 9 is read that way. Then G3.

Three coders, from today:

| Coder | Start now | Then |
| --- | --- | --- |
| 1. Appearance | P2.3c1 (three parts), then P2.3c2 (two parts) | The first object-spine row that is free. Take and rez (P2.8d, P2.8e) once P2.8a is `DONE`. |
| 2. Children | Q79c2a, then Q79c2b, then P2.11b (two parts), then P2.11c | Object cross in (P2.10c) once P2.10b is `DONE`, if coder 3 has not taken it. |
| 3. Motion, then parcels, then the spine | P2.3b1 (two parts), then P2.3b2 | P2.6b's Q61b, then Q61c. When Q65 is marked `DONE`, this coder takes P2.5c if coder 1 is still in bakes, then stays on the spine: P2.8a, P2.8b1, P2.8b2, and from there P2.9a and P2.10b. P2.5b can fill a gap beside P2.5c. |

Whoever is free after the status close also picks up the suggested OAR row. It is one day and waits on nothing open.

These can run beside the three lanes and are not predecessors of them:

- P2.10a2, the seated two-C# capture, any time.
- P2.6a3a2, the Local groups connector, any time. It does not unblock the Robust-grid checks.
- P2.8c part 2 (duplicate) beside P2.9a, once P2.8b2 has landed and part 1 of P2.8c has the link path.
- P2.9b's MySQL and PostgreSQL parts, in order, as soon as P2.9a part 1 has proved the writer. The gate still waits for both.

P2.5d stays out of this schedule.

The first person free when Q65 flips to `DONE` should take P2.5c. Waiting for a named coder to finish bakes or children leaves the long pole idle.

## 5. Open questions

1. Is G3's one-sentence list (log in, walk, see another avatar, build, chat, teleport to and from the C# region, Firestorm and Aurora) the whole gate, or does the person also do the PLAN items that sentence leaves out (stored objects, inventory take and rez, appearance, the neighbour in view, an object crossing, and an OAR load with terrain-height standing only)? If the sentence is the whole gate, P2.12's checklist is that sentence and the OAR row is PLAN scope that G3 does not exercise. If the PLAN sentence is the gate, section 1 is the script P2.12 writes into [MANUAL_CHECKS.md](../MANUAL_CHECKS.md).

2. Does "cross a border" mean D38's pair (teleport both ways, plus child agents so the C# neighbour is in view) together with a riderless object crossing (`SupportsHeldCrossing` false)? Walking across the border stays deferred under that reading. The other reading adds a walking cross, which has no P2 row and which D38 postponed.

3. Is the person's check Firestorm only, or Firestorm and Aurora as the G3 row is written? The script in section 1 is the same either way. The G3 evidence names one viewer or both.

4. Does the Robust-grid gate require the Local groups connector (P2.6a3a2 / Q86)? The harness grid uses Robust, and Q60c3 already delivers the overlay from remote memberships. Q86 is inside the written P2.12 range and is `READY` now. Leaving it out of the gate lets the three coders skip it. Keeping it in the range holds P2.12.

5. Does G3 wait on the seated two-C# fixture (P2.10a2 / Q76b)? Held crossing is off (D39). P2.10b's evidence is the riderless capture. P2.10a2 is inside the written P2.12 range and can be captured in parallel. Dropping it from the range lets P2.12 ignore it.

6. Does take include `DeRezAction.Return` and `GodTakeCopy`, or only `Take`, `TakeCopy`, and `Delete`? The proposed P2.8d split covers the three the row names. Return is the land path, and Phase 2 writes only the default parcel.

7. Does rez include a coalesced object and a rez from a prim's inventory, or only one user-inventory object onto the terrain? The proposed P2.8e split does the single object and the coalesced path. The prim-inventory path (`fromTaskID` non-zero) waits on this answer.

8. Is a worn attachment part of the appearance check, or is the check the viewer-side bake only (P2.3c1 and P2.3c2)? P2.0f sent no attachment block. `Client_OnAvatarNowWearing` exists. Attachment rez is not in either reading of this question.

9. Must the manager mark a finished split parent `DONE` before a coder may take the next `READY after` item? That is how rule 1 reads. It covers Q65, which blocks the object spine, and Q40, Q46, Q53, and Q55, whose task rows are already `DONE` while the queue status still says `SPLIT`. It also covers Q81, which is `READY after Q40 to Q80 and Q82 to Q85`. If a `SPLIT` parent whose leaves are `DONE` counts as done for rule 1, say so and the spine starts. If it does not, the status close in section 4 is the first thing on the critical path.
