# Phase 2 gate map

Planning note, 26 November 2026, revised the same day after the gate-map review and after the P2.5a and P2.6a parents closed. This file maps what is left before a person can log Firestorm into a Ferroan region on a grid where Robust and the neighbours are still C#, and do the Phase 2 scope in [PLAN.md](../PLAN.md). It proposes splits. It does not add those rows to [TASKS.md](../../TASKS.md) or [WORK_QUEUE.md](../../WORK_QUEUE.md). The manager assigns any new ids.

The C# citations are Pin8, `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`, read at that commit. Paths are relative to that tree. Where an older task cell cites `1d7abb8337` and the line has moved, this file uses the Pin8 line.

Standing decisions: [D38](../DECISIONS.md) (the walking stand-in, next door as teleport both ways plus child agents, viewer-side bakes, all three region stores) and [D39](../DECISIONS.md) (scene state on the frame thread, no held object crossing, parcels read and only the default parcel written, persistence timing, materials passed through). Phase 2's OAR load is a full Pin8 `load oar`: settings, parcels, terrain, objects and assets. The avatar stands at terrain height only, because D38's stand-in walks through prims. Saving an archive, and IAR round trips, stay in Phase 5.

The queue still records two coders (rule 10). Section 4 is the parallel map for three, which is what this note was asked for.

## 1. The gate

[PLAN.md](../PLAN.md) states the Phase 2 scope: the LLUDP circuit, CAPS and the event queue, the scene, object and inventory updates, permissions, appearance and baking, teleports and crossings to and from C# regions, and loading OAR files with terrain-height standing only. That OAR phrase is the full load in section 3, with standing at terrain height only. After P2.0j, the region work is P2.0e2, P2.0e3, P2.1a through P2.12, then G3.

G3, in [TASKS.md](../../TASKS.md), is narrower than that sentence. It says the maintainer logs in with Firestorm and Aurora, walks, sees another avatar, builds, chats, and teleports to and from the C# region next door, following a Phase 2 section in [MANUAL_CHECKS.md](../MANUAL_CHECKS.md). That section does not exist yet. P2.12 writes it. [MANUAL_CHECKS.md](../MANUAL_CHECKS.md) says the Phase 2 gate is G3, after the later region work. G2, the first-login check, is still unrecorded. Later P2 rows do not wait on G2.

The table below is the person's script for the PLAN scope. Question 1 asks whether G3's shorter sentence replaces it. Automated evidence is a bot scenario, a capture compare, or a live run in [LIVE.md](../LIVE.md). [TESTING.md](../TESTING.md) keeps the harness as protocol evidence. A passing run is not the Firestorm result. The person records viewer versions in the G3 row. Ferroan has no physics switch: it always runs the walking stand-in. A C# walk, animation, or crossing capture that the compare uses is taken with `--physics basicphysics`, because the harness default is ubODE and the terse values differ.

`cargo xtask live run all` runs the names in the bot's scenario list. Today those pass lines include `login: pass`, `walk: pass (forward movement, observer kills=1, leaving viewer kills=0)`, `chat: pass`, `build: pass`, `teleport: pass` with the landing position, `texture: pass`, and `inventory: pass`. `cargo xtask live compare region` prints `region: match` when both a C# home and a Ferroan home print `login: pass`, `chat: pass`, `teleport: pass`, `teleport neighbour: pass (1001,1000 and back)`, and `live: run passed`. Dedicated probes that `all` leaves out, and that later rows add, are named in the table. P2.12 asks the Ferroan `all` run to print the same lines as C# for walk, avatars, build, link, take, rez, store swap, and object cross, plus teleport both ways and the neighbour in view.

| What the person does | Automated evidence that must already pass |
| --- | --- |
| Log in through C# Robust and arrive standing on the Ferroan region | Already passing: `cargo xtask live run login grid --region ferroan` prints `login: pass`, and `compare region` includes it. Circuit, seed, event queue, arrival, and logout are P2.0a through P2.0d4 and P2.0d6. `cargo xtask live run caps grid --region ferroan` covers the seed and the held empty poll. |
| See the region and its terrain, standing at terrain height | Already passing: P2.0e `LayerData` matches the stored heightmap and `AgentMovementComplete` puts the avatar at that height. P2.0e2 and P2.0e3 write and load the default terrain on SQLite, MySQL, and PostgreSQL. The walk line lands at that height. |
| See objects stored in the region | Full updates of a handed-off store already pass: `cargo xtask live run store-handoff grid --physics basicphysics` and `cargo xtask live run prim-load grid --physics basicphysics` (P2.5a4a and P2.5a4b). Before this Firestorm check, P2.5b part 1 must match compressed updates against a Pin8 capture taken with `SupportViewerObjectsCache` on, the default (`LLUDPServer.cs` 361). Those blocks follow the region's `SupportViewerObjectsCache` and the group's `IsViewerCachable`. A bot that replies `0x7` still receives them. The P2.5a captures show none because they were taken with the setting off. P2.5b part 2 must match the arrival probes. With culling off, the default, that decision is `ScenePresence.SendInitialData`: the 0x2 bit from `GetViewerCaps` chooses `SendUpdateProbes` or a full update. P2.5c's properties part must match `ObjectProperties`. Distance culling stays off unless the capture turns it on, and that capture is P2.5c's culling part. |
| Walk | `walk: pass` already, from the stand-in (P2.2b1, P2.2b2). Before the person walks, P2.3b1's state tests and P2.3b2's observer must show walk then stand with the same animation UUIDs and sequence numbers as a basicphysics C# region. The existing `walk: pass` line does not check those UUIDs. |
| See another avatar | Already passing: `avatars: pass` (P2.3a) and `UUIDNameReply` (P2.3d). Before the person looks, P2.3c2's observer must receive the same `AvatarAppearance` texture entry C# sends, and P2.3b2's animation packets must match. Without P2.3c2 the other avatar stays grey or Ruth. |
| Chat | Already passing: `chat: pass` (P2.0g), including the second account on channel 0 and DEBUG_CHANNEL type 6. P2.0g is DONE. The Ferrobot table in [LIVE.md](../LIVE.md) still lists Ferroan local chat as pending (P2.0g). P2.12 closes that line in the gate report. It is a docs fix, and it is not a new chat row. |
| Edit and rez an object | P2.8a through P2.8c must pass first: rez, select, move, rotate, texture, link, and unlink, with `build: pass` on the Ferroan region and a new `link: pass`. Rez from inventory is P2.8e. Permissions for rez, edit, move, delete, take, link, and copy are P2.7a through P2.7c, already DONE, and these rows call them. |
| Build, restart, and read the same store from C# | P2.9a part 2 is the SQLite store swap: build on Ferroan, stop, start Ferroan again and see the build, then start C# on the same file and read the same objects. P2.9b repeats that swap on MySQL and on PostgreSQL. D38 puts all three stores on the gate. P2.12 names `store swap`. |
| Use inventory | The window and folder fetch already pass: `inventory: pass` and the fetch caps (P2.0d7). Take into inventory and delete to trash are P2.8d (`take: pass`, asset XML compared with C#). The prim is gone from the store only after P2.9a's `RemoveObject`. Rez of that item is P2.8e, both directions. |
| Change appearance | P2.0f already sends the unbaked `AvatarAppearance` and avatar `ObjectUpdate`. P2.3c1 must match wearables, cached-texture, and `UploadBakedTexture` replies with C# for the same account. The upload compare is the two LLSD replies, the remote-address check, and the cached local asset. P2.3c2 must store through `setavatar` and broadcast the baked texture entry. `ServerSideBakingModule` stays out (D38). |
| Teleport to a C# region and back | The hop without a child already passes: `teleport neighbour: pass (1001,1000 and back)` and `cargo xtask live run hop grid --region ferroan` / `neighbour grid --region ferroan` (P2.0i). After Q79c2b, re-run that neighbour teleport with the child open. At Pin8 an in-view hop reuses the child's seed and sends no `EnableSimulator` (`EntityTransferModule.cs` 1045–1060, 1137–1141). `NeedsClosing` keeps the old root as a child (1292). Q79c1 already touched `GetChildSeed`. The passing line above does not cover that re-run. |
| Cross a border | Read as D38's next-door pair plus an object crossing. The neighbour is in view while the avatar stays root at home: P2.11a's `cargo xtask live run child-view grid --physics basicphysics` with Ferroan as home, then P2.11b the other way, then P2.11c's throttle fields on those same messages. An object crosses both ways with `SupportsHeldCrossing` false: P2.10b and P2.10c, `object cross: pass`, against the riderless P2.10a capture. Walking across the border stays deferred (D38). Next door is teleport both ways plus child agents, and P2.12 already names object cross. |
| Stand on an OAR-loaded region | P2.0e4, proposed in section 3. A full Pin8 `load oar` of the same archive: settings, parcels, terrain, objects and assets. The avatar stands at that terrain height, and walks through the loaded prims, because the stand-in does. `LayerData`, the loaded objects, parcels, settings and assets match a C# region that loaded the same archive. |

P2.12 is the report that gathers those lines, adds the Phase 2 section to [MANUAL_CHECKS.md](../MANUAL_CHECKS.md), and stops. G3 is the person, after that section exists. A Ferrobot session that watches another avatar walk and build is part of P2.12's evidence, as far as the pinned Ferrobot API allows. Region IM stays out. The Ferrobot IM line is still pending and is not a Phase 2 gate check. The stale Ferrobot local-chat line is the docs fix named in the chat row.

## 2. Rows that are not DONE

Twenty-five P2 rows are not `DONE`. Each note says what the row still needs, what it depends on, whether the gate waits for it, and whether it is more than about a day. A proposed split is a proposal. The parent stays open until every part is `DONE`, except a follow-up the closing note leaves open. P2.6a3a2 is that follow-up: P2.6a3 is `DONE`, and the follow-up stays `TODO`. The manager assigns the queue ids.

### Closed parents

These six parents are `DONE`. The first draft of this map still had them `BLOCKED` and treated every one of them as the status close the object spine was waiting on. The object spine waited on P2.5a / Q65 only. P2.5a3, P2.5a3c and P2.5a4 (Q65c, Q65c3, Q65d) gate nothing but P2.12's range. P2.6a was P2.9a's task dependency. P2.6a3 was P2.6b's task dependency. Both of those dependencies are satisfied. Nothing in this table is left to close before a coder starts.

| Row | Queue | What closed | What it gated |
| --- | --- | --- | --- |
| P2.5a | Q65, `DONE` (split into Q65a–Q65d, all done) | Leaves P2.5a1, P2.5a2, P2.5a3a, P2.5a3b, P2.5a3c1, P2.5a3c2, P2.5a4a, and P2.5a4b, including the store handoff and the 500-part load. | The object spine. Q66 and Q67 are `READY after Q65`. Q65 is `DONE`, so P2.5b and P2.5c can start. |
| P2.5a3 | Q65c, `DONE` | Q65c1, Q65c2, Q65c3a, and Q65c3b. | P2.12's range only. No other row depends on it. |
| P2.5a3c | Q65c3, `DONE` | The mixed Task drain and the material-override datagram. The `RenderMaterials` cap stays in P2.5d. | P2.12's range only. |
| P2.5a4 | Q65d, `DONE` | Q65d1 and Q65d2. | P2.12's range only. |
| P2.6a | Q60, `DONE` (split into Q60a–Q60c, all done) | Q60a, Q60b, Q60c1, Q60c2, and Q60c3. P2.6a3a2 stays open as a follow-up, outside this close. | P2.9a's task dependency. Q74 itself says `READY after Q70` only. That task dependency is now satisfied. |
| P2.6a3 | Q60c, `DONE` (split into Q60c1–Q60c3, all done) | The overlay on the Land throttle. The task cell says P2.6a3a2 stays open as a follow-up. The parent does not reopen for it. | P2.6b's task dependency. Q61b is `READY after Q61a`. Q61a is `DONE`. Q61c is `READY after Q61a, Q61b and Q60c3`, and Q60c3 is `DONE`. |

P2.0d5 is a stale status, not remaining work. The cell is `TODO`. Its own evidence is [PR #90](https://github.com/KanikaBreed/Ferroan/pull/90), implementation `1674cf6`, with `cargo xtask http status-verify` in the live job. Q10h is `DONE (#90)`. It depends on P2.0d3, which is `DONE`. It is outside P2.12's dependency range. Critical path: no. The manager can mark the cell `DONE` from the evidence already in the row.

### Off the gate

**P2.5d.** `TODO`. Not queued. The `RenderMaterials` cap and the `needMaterials` path (`Source/OpenSim.Region.OptionalModules/Materials/MaterialsModule.cs`, 1458 lines at the older pin; the row still says a few days, as a guess). Depends on P2.5a, which is `DONE`. D39 keeps Phase 2 to material ids in the texture entry and extra params. The negotiated UDP material send is already in P2.5a3c2. Critical path: no. Leave it unsplit until someone queues it after the gate.

### Already split

These stay as the approved notes. This file does not split them again.

**P2.6b / Q61.** `TODO`. Parcel properties over the event queue on arrival, on `ParcelPropertiesRequest` (`LLClientView.cs` 10874, `SendLandProperties` 6757), and when the parcel changes while walking. Parcel editing stays in Phase 5 (D39). The task dependency is P2.6a3 and P2.2b2. P2.6a3 is `DONE` (Q60c1–Q60c3), and P2.2b2 is `DONE`, so nothing else must close before the remaining parts. Q61a is `DONE` and published the capture. Q61b (`READY after Q61a`) encodes `SendLandProperties` and matches each captured element, about a day, and can start. Q61c (`READY after Q61a, Q61b and Q60c3`) delivers the three paths and compares live with Pin8, about a day. Q60c3 is `DONE`, so Q61c waits on Q61b. The gate waits for P2.6b, because P2.12's range includes it. It is not the long pole. The person's script in section 1 does not name the About Land dialog.

**P2.11a / Q79.** `BLOCKED`. Ferroan opens a child on the C# neighbour so that neighbour is in view (D38). Depends on P2.0i and P2.2b2, both `DONE`. Q79a, Q79b, and Q79c1 are `DONE`. There is no `P2.11a3b` row on this branch. The approved Q79c2 note names the two parts that finish the parent:

1. **Q79c2a (P2.11a3b1),** `READY after Q79c1`, about a day. Finish every child claim once, and close children on a far hop. No frame loop and no live grid. Pin8 `CloseChildAgents` is `ScenePresence.cs` 5060–5071. `InformClientOfNeighbourAsync` sleeps 500 ms after a successful `CreateAgent` and before `EnableSimulator` (`EntityTransferModule.cs` 2781; 2780 is the comment). `GetChildAgentsToClose` is `EntityTransferModule.cs` 1082 (V1) and 1319 (V2). `CloseChildAgents` runs at 1280 (V1) and 1474 (V2).
2. **Q79c2b (P2.11a3b2),** `READY after Q79c2a`, about a day. Drive the neighbour set from the frame. Arrival calls `EnableChildAgents` at `ScenePresence.cs` 2575 and 4398. The child half of `CheckForSignificantMovement` is 4750–4828. Evidence is `cargo xtask live run child-view grid --physics basicphysics` with Ferroan as home, against the Q79a capture. The section 1 teleport row re-runs `teleport neighbour` with that child open.

P2.11a stays `BLOCKED` until both are `DONE`. The gate waits for it. Walking across the border stays deferred.

### Rows that need a split

Each proposal is more than about a day at Pin8 `c401bceecd`, or the review found the row was sized past a day. The parent stays `TODO` until every part is `DONE`.

**P2.3b1.** `TODO`. The animation state machine. Depends on P2.2b2 and P2.3a, both `DONE`. Q49 is `READY after Q46b and Q48`, and both are `DONE`, so this can start now. The gate waits for it. The person's walk and the other avatar's motion use it. It sits behind the object spine in section 4. `ScenePresenceAnimator.cs` is 885 lines, `AnimationSet.cs` is 361, and `DefaultAvatarAnimations.cs` is 124, the same sizes the row names.

1. **Movement states** (about one day). `DetermineMovementAnimation` (`ScenePresenceAnimator.cs` 260–570), `UpdateMovementAnimations` (576–594), and `TrySetMovementAnimation` (187). The stand-in supplies "on ground" in place of `IsColliding`. Unit tests for walk, run, fly, fall, land, and stand, matching the states a basicphysics C# region reaches.
2. **Worn and overridden animations** (about one day, after part 1). `AddAnimation` (81 and 100), `RemoveAnimation` (122 and 156), `ResetAnimations` (168), and `AnimationSet.cs`. With nothing worn, the movement state from part 1 still wins.

`GenerateRandomAnimation` (625) is outside the six transitions the row names. `SendAnimPack` (823, 838, and 854) is P2.3b2.

**P2.3c1.** `TODO`. Viewer-side bakes, caps and circuit only. Depends on P2.0f, `DONE`. Q51 is `READY after Q18a`, which is `DONE`, so this can start now. The gate waits for it. `AvatarFactoryModule.cs` is 1518 lines. `UploadBakedTextureModule.cs` is 225.

1. **Wearables** (about one day). `AgentWearablesRequest` / `AgentWearablesUpdate` through `Client_OnRequestWearables` (`AvatarFactoryModule.cs` 1275). Replies compared with C# for the same account.
2. **Cached textures** (about one day, after part 1). `Client_OnCachedTextureRequest` (1437) and `ValidateBakedTextureCache` (549). The bot's cached-texture query matches C#.
3. **UploadBakedTexture** (about one day; it does not need part 1, so it can start beside part 1). `UploadBakedTexture` (`UploadBakedTextureModule.cs` 113–148) and `BakedTextureUploader.process` (189–220). `SaveBakedTextures` (`AvatarFactoryModule.cs` 315) is the OSSL appearance-to-notecard path (`OSSL_Api.cs` 3587), and it is not this upload. The upload returns an uploader URL, then stores the bake under a random id as a temporary local asset in the region's asset cache. That id never reaches the asset service, so the evidence does not compare asset ids. Compare the two LLSD replies, the remote-address check, and the cached local asset.

The row's guess stands: confirm which bake path this grid's Firestorm OpenSim build uses while doing part 1. `ServerSideBakingModule` stays out.

**P2.3c2.** `TODO`. `AgentSetAppearance`, store, and broadcast. Depends on P2.3c1 and P2.3a. Q52 waits on Q51 and Q48. Q48 is `DONE`. The gate waits for it. Without it, Firestorm users see each other grey or as Ruth.

1. **Accept and store** (about one day, after P2.3c1). `Client_OnSetAppearance` (1296), `SetAppearance` (195, and the overloads at 201 and 221), `SaveAppearance` (884), and `SetAppearanceAssets` (940). The avatar row matches C#'s `setavatar` result. Bot command: `AgentSetAppearance`.
2. **Broadcast** (about one day, after part 1). `SendAppearance` (274). The observer receives the same `AvatarAppearance` texture entry C# sends.

**P2.5b.** `TODO`. Compressed and cached updates. Depends on P2.5a, which is `DONE`. Q66 is `READY after Q65`, and Q65 is `DONE`, so it can start now, beside P2.5c. P2.8a depends on P2.5c's properties part, so this row does not lengthen the object spine. The gate waits for it on the Firestorm "see stored objects" check. `SupportViewerObjectsCache` defaults to true (`LLUDPServer.cs` 361). The P2.5a captures were taken with that setting off.

1. **Compressed blocks** (about one day). Independent of part 2. `useCompressUpdate = grp.IsViewerCachable` is set only under `viewerCache`, the region's `m_supportViewerCache` (`LLClientView.cs` 5020, 5206–5207, 5295). `IsViewerCachable` is `SceneObjectGroup.cs` 199–210. A bot that replies `0x7` still gets compressed blocks from a default C# region. No bot change for this part. Evidence: bytes compared with a Pin8 capture of the same stored objects taken with `SupportViewerObjectsCache` on. `CreateCompressedUpdateBlock` is `LLClientView.cs` 7892–8133 and `CreateCompressedUpdateBlockZC` is 8134.
2. **Cached probes** (about one day, beside part 1). The probe CRC is the stored `PseudoCRC` (`LLClientView.cs` 5569; `SQLiteSimulationData.cs` 1877 and 2308). It is a stored column. With culling off, the default, the arrival decision is `ScenePresence.SendInitialData` 4361–4384. `cacheEmpty = (flags & 2) != 0` comes from `GetViewerCaps` (`LLClientView.cs` 13746–13754). That chooses `SendUpdateProbes` (`SceneObjectGroup.cs` 3316) or `SendFullAnimUpdateToClient`. The 0x2 test at `LLClientView.cs` 5948, and the clear at 5963, sit inside `CheckGroupsInView`, which returns unless `ObjectsCullingByDistance` is on (5859). Those lines are the culling path in P2.5c, not this part. `ObjectUpdateCached` header at 4989, the cached copy at 5532, and `HandleRequestMultipleObjects` at 9744 (registered at 8580). A bot mode with the 0x2 bit clear. The exact Firestorm flag value comes from a capture. The bot sends `0x7` today.

**P2.5c.** `TODO`. Interest list and properties. Depends on P2.5a, which is `DONE`. Q67 is `READY after Q65`, and Q65 is `DONE`, so this can start now. The gate waits for it. P2.8a waits on the properties part only. `ObjectsCullingByDistance` defaults to false (`Scene.cs` 806, read at 1203–1204; `RegionServerDefaults.ini` 2764). With that default, a Pin8 viewer does not see objects leave view by distance. The C# capture for part 2 turns culling on. The shipped default stays off, so the ordinary Firestorm check needs properties and kills, and the enter-and-leave check is the capture with culling on.

1. **Properties** (about one day). `ProcessEntityPropertyRequests` (`LLClientView.cs` 6222–6395) and the two block builders (6396, 6431). `SendObjectPropertiesFamilyData` (6173) and `SendObjectPropertiesReply` (6203). `SendKillObject` (2374) stays with this part, because a delete still has to tell the viewer. Evidence: property bytes match, and a killed object leaves. P2.8a starts after this part.
2. **Culling** (about one day, beside part 1 once the kill path exists). `CheckGroupsInView` (`LLClientView.cs` 5857–5973), the update-loop cull (5160–5180), and the `SendInitialData` branch (`ScenePresence.cs` 4349–4358). Evidence: with culling on, the observer sees objects enter and leave view as on a C# region with the same setting on.

**P2.8b2.** `TODO`. Edit, appearance and flags. Depends on P2.8b1. Q70 waits on Q69. The gate waits for it. Link, persistence, and the rest of `build: pass` wait on part 2. The five handlers are short. The scene side is about 400 lines, and each change needs a C# compare, so the row is two parts.

1. **Texture, shape and extra params** (about one day, after P2.8b1). `HandleObjectShape` (`LLClientView.cs` 9674), `HandleObjectExtraParams` (9711), and `HandleObjectImage` (9861; the next handler starts at 9891). Scene side: `SceneObjectPart.UpdateTextureEntry` (5295–5414), `UpdateShape` (5205–5243), and `SceneGraph` 1736–1939. Evidence: the texture check, and the shape and extra-param updates compared with C#.
2. **Flags and permissions** (about one day, after part 1). `HandleObjectFlagUpdate` (9818; the physical flag is stored and does nothing until Phase 3) and `HandleObjectPermissions` (10023) through `Scene.HandleObjectPermissionsUpdate` (`Scene.cs` 5389). Scene side: `SceneObjectPart.UpdatePrimFlags` (4930–5038), `UpdatePermissions` (4823–4882), and `SceneObjectGroup.UpdatePermissions` (5445–5483). Evidence: the rest of `build: pass`. Link and persistence wait on this part.

**P2.8c.** `TODO`. Link, unlink, and duplicate. Depends on P2.8b2. Q71 is `READY after Q70`. The gate waits for it. P2.12 names `link: pass`.

1. **Link and unlink** (about one day). `HandleObjectLink` (`LLClientView.cs` 9624), `Scene.Inventory.cs` `LinkObjects` (3679 and 3690), `SceneGraph.cs` `LinkObjects` (1940), `SceneObjectGroup.cs` `LinkToGroup` (4199), and `DelinkFromGroup` (4489, with the other overloads at 4445, 4462, and 4614). `link: pass` for a two-prim link and an unlink. The prims-per-linkset limit applies.
2. **Duplicate** (about one day, after part 1). `SceneGraph.cs` `DuplicateObject` (2188), `Scene.cs` `DuplicateObject` (3778), and `HandleObjectDuplicate` (`LLClientView.cs` 9726). The observer sees the copy.

**P2.8d.** `TODO`. Take and delete to trash. Depends on P2.8a and P2.S1. P2.S1 is `DONE`. The delete part also depends on P2.9a, because "gone from the store" is `RemoveObject`. Q72 is `READY after Q68 and Q57`, and Q57 is `DONE`, so the take part can start when rez-and-select lands. It does not wait for texture edit, and it does not wait for P2.9a. The gate waits for the row. This note adds the P2.9a dependency for the delete part. The manager applies it when the row is split.

1. **Take and TakeCopy** (about one day, after P2.8a). `DeRezAction.Take` (`Scene.Inventory.cs` 2944) and `TakeCopy` (2935) through `InventoryAccessModule.cs` `CopyToInventory` (442). `AsyncSceneObjectGroupDeleter.CopyToInventory` is the same writer (`AsyncSceneObjectGroupDeleter.cs` 130). The asset XML matches what C# writes, and the new item is in inventory. `take: pass`.
2. **Delete to trash** (about one day, after part 1 and P2.9a). `UseTrashOnDelete` defaults to true (`Scene.cs` 271; `RegionServerDefaults.ini` 399). `DeRezAction.Delete` (`Scene.Inventory.cs` 2958) adds the object to `takeDeleteGroups` (2962–2963). That goes through `AsyncSceneObjectGroupDeleter.DeleteToInventory` (`Scene.Inventory.cs` 3018) into `CopyToInventory` (`AsyncSceneObjectGroupDeleter.cs` 130). The other viewer gets `KillObject` (P2.5c), and the prim is gone from the store (`RemoveObject`, P2.9a).

`DeRezAction.Return` (2970) and `GodTakeCopy` (2951) are question 6. The two parts above cover the actions the row names.

**P2.8e.** `TODO`. Rez from inventory. Depends on P2.8d and P2.S2. P2.S2 is `DONE`. The take part of P2.8d is enough for this row to start; the delete-to-trash part can follow. Q73 waits on Q72 and Q58. Q58 is `DONE`. The gate waits for it. P2.12 names rez.

1. **One user-inventory object** (about one day, after P2.8d's take part). `Scene.Inventory.cs` `RezObject` (3269). The `fromTaskID` zero path (3278–3285) calls `InventoryAccessModule.RezObject` (892, and the overloads at 902, 933, and 944). The ray is placed on terrain (D38). A no-copy item is removed. Rez an item the C# region took, and the reverse, for a single object.
2. **Coalesced rez** (about one day, after part 1). `GetObjectsToRez` (`Scene.Inventory.cs` 3138) and the next-owner masks. P2.S2 and P2.S3 already cover the XML reader and the coalesced bytes.

Rez from a prim's inventory (`fromTaskID` non-zero, 3288) is question 7.

**P2.9a.** `TODO`. SQLite persistence. Depends on P2.8b2 and P2.6a. P2.6a is `DONE`. Q74 is `READY after Q70`. The gate waits for it. This is the second half of the store swap, and D38 puts every region store on the gate through P2.9b. Section 1's persistence step is this row plus P2.9b.

1. **One prim, forced** (about one day, after P2.8b2). `SQLiteSimulationData.cs` `StoreObject` (534) writes through `addPrim` (2598). `RemoveObject` is 557. `fillPrimRow` is 2132–2326. The next method is `fillItemRow` (2327). `fillShapeRow` is 2549, further down, and this part still cites it because the shape columns are stored. `StorePrimInventory` is 2632. `ForceSceneObjectBackup` (`Scene.cs` 2235) calls `ProcessBackup` (`SceneObjectGroup.cs` 3162). A Ferroan stop and start on the same SQLite file keeps the prim. `fillPrimRow` is a long column writer. If the oracle dump overruns the day, split that writer off on its own and keep the forced-backup check with it.
2. **Periodic backup and the default parcel** (about one day, after part 1). `Backup` (`Scene.cs` 2170), `UpdateStorageBackup` (2142), the `m_dontPersistBefore` / `m_persistAfter` window (318, 320, read at 1071–1074), and `Backup(true)` on shutdown (1615). `StoreLandObject` (1018) writes only the default whole-region parcel. Evidence is the second half of the store swap: build on Ferroan, stop, start Ferroan on the same file and see the build, then start C# on that file and read the same objects.

**P2.9b.** `TODO`. MySQL and PostgreSQL persistence, the same rules as P2.9a, plus `cargo xtask regionstore cross` for writes. Depends on P2.9a and P2.4b. P2.4b is `DONE`. Q75 waits on Q74 and Q55. Q55 is `DONE`. The gate waits for it, because D38 requires all three databases at the gate. A SQLite-only Firestorm session can be rehearsed earlier. The gate itself waits for this row.

1. **MySQL** (about one day, after P2.9a). `MySQLSimulationData.cs` `StoreObject` (130), `RemoveObject` (260), `StorePrimInventory` (1967), and `StoreLandObject` (712). The store swap on MySQL, and the MySQL half of `regionstore cross` for writes.
2. **PostgreSQL** (about one day, after part 1, or beside it once part 1 has proved the writer). `PGSQLSimulationData.cs` `StoreObject` (259), `RemoveObject` (448), `StorePrimInventory` (483), and `StoreLandObject` (758). The store swap on PostgreSQL, and the PostgreSQL half of the same cross.

**P2.11b.** `TODO`. Ferroan accepts a child `CreateAgent` from the C# neighbour, which it refuses today, and an `AgentPosition` PUT that checks the session before it changes state. Depends on P2.11a. Q80 is `READY after Q79c2b`. The gate waits for it (D38, both directions). The throttle scale inside the position update is P2.11c, not these parts.

1. **Accept the child** (about one day, after Q79c2b). The child branch of `NewUserConnection` (`Scene.cs` 4117). The child-versus-root log is at 4137. An existing child is reused at 4211. The circuit stays a child. Evidence: a C# neighbour's child POST is accepted, and the bot does not become root.
2. **Position and the full child update** (about one day, after part 1). `IncomingUpdateChildAgent(AgentPosition)` (`Scene.cs` 4848) refuses a session mismatch at 4860 and calls `ScenePresence.UpdateChildAgent(AgentPosition)` (5141). `IncomingUpdateChildAgent(AgentData)` (4773) refuses a session mismatch at 4819 and calls `UpdateChildAgent(AgentData)` (5125). Evidence: the child pose matches the capture, and a wrong session is refused. Part 2 may apply throttles at factor 1. P2.11c replaces that with the distance scale.

### About a day, leave whole

**P2.3b2.** `TODO`. Animation packets and viewer-started animations. Depends on P2.3b1. Q50 waits on Q49. The gate waits for it. At this pin the packet send is `LLClientView.SendAnimations` (4666), fed by `ScenePresence.SendAnimPack` (4642) and `ScenePresenceAnimator.SendAnimPackToClient` (838). `HandleAgentAnimation` is `LLClientView.cs` 9466. The older cell's `ScenePresence.SendAnimations` (4666) is this `LLClientView` method. Evidence: the observer sees walk, then stand, with the same UUIDs and sequence numbers as C# running basicphysics. About a day. No split.

**P2.6a3a2.** `TODO`. Read a viewer's group memberships from the Local SQLite `os_groups` connector. This is a follow-up outside the Q60c split. P2.6a3 is `DONE`, and this row stays open. Its id sits under P2.6a3a, and that nesting does not reopen the parent. Depends on P2.6a3a, `DONE`. Q86 is `READY after Q60c1`, which is `DONE`, so this can start now. Pin8's type is `GroupsServiceLocalConnectorModule` in `Addons/OpenSim.Addons.Groups/Local/GroupsServiceLocalConnectorModule.cs` (class at 41). `GetAgentGroupMemberships` is at 298 and forwards to the groups service. Q60c1 refuses that connector at startup until this row lands. The harness grid uses Robust, and Q60c1 already reads remote memberships. Evidence: member, non-member, and owner cases against Pin8 on the same SQLite store. About a day. No split. The Robust-grid Firestorm login does not wait for it. It sits inside P2.12's TASKS range (P2.1a through P2.11c). It does not sit in Q81's queue range, which is Q40 to Q80 and Q82 to Q85. Question 4 is about those two ranges.

**P2.8a.** `TODO`. Rez a new prim and select it. Depends on P2.5c and P2.7c. P2.7c is `DONE`. The proposed P2.5c split lets this row start after the properties part. Q68 waits on Q67 and Q64. Q64 is `DONE`. The gate waits for it. `HandleObjectAdd` (`LLClientView.cs` 9659), `Scene.AddNewPrim` (`Scene.cs` 2682), `SelectPrim` and `DeselectPrim` (`Scene.PacketHandlers.cs` 196 and 272), `HandleObjectName` (10012), and `HandleObjectDescription` (10001). The ray is placed on terrain (D38). Evidence: the first half of `build: pass` on the Ferroan region, and the observer sees the new prim. About a day. No split.

**P2.8b1.** `TODO`. Edit, transform. Depends on P2.8a. Q69 waits on Q68. The gate waits for it. P2.10b also waits on it. `HandleMultipleObjUpdate` (`LLClientView.cs` 12623), `UpdatePrimScale` (`SceneGraph.cs` 1545), `UpdatePrimGroupRotation` (1645 and 1664), and `UpdatePrimGroupPosition` (1700–1724). Evidence: the move half of `build: pass`, plus a rotate check, with the observer's updates compared with C#. About a day. No split.

**P2.10a2.** `TODO`. The seated-avatar object-crossing fixture, two Pin8 C# regions only. Depends on P2.10a, `DONE`. Q76b is `READY after Q76`, which is `DONE`, so this can start now. It records the rider's agent transfer beside the object post, and what each viewer receives. The riderless path is already captured. Pin8's access check at `EntityTransferModule.cs` 1764 uses the agent's own UUID. A teleport while sitting stands the avatar up at `EntityTransferModule.cs` 1001 before the transfer. Held crossing stays off (D39). D39's consequence is that riders and script state wait for Phases 3 and 4. About a day of capture. No split. The Ferroan object-cross code does not wait for it. P2.10b does not depend on it. It sits inside P2.12's written TASKS range. Question 5.

**P2.10b.** `TODO`. Object crossing, Ferroan to C#. Depends on P2.10a, P2.S1, P2.8b1, and P2.0i. P2.10a, P2.S1 and P2.0i are `DONE`. P2.8b1 is not. Q77 waits on Q76, Q57, Q69, and Q18c. The gate waits for it. The `AbsolutePosition` setter (`SceneObjectGroup.cs` 883) calls `StartCrossing` (900 and 960) when the new position is outside the region. `CrossPrimGroupIntoNewRegion` (`EntityTransferModule.cs` 2968) posts through `SimulationServiceConnector.CreateObject` (487) and then deletes the local object (2988). `state` passes through unchanged (D39). Evidence: the region-to-region bodies match the riderless P2.10a capture, and the C# region shows the object. About a day. No split.

**P2.10c.** `TODO`. Object crossing, C# to Ferroan. Depends on P2.10b and P2.S2. Q78 waits on Q77 and Q58, and does not wait on P2.9a. The gate waits for it. `ObjectSimpleHandler.DoObjectPost` (`Source/OpenSim.Server.Handlers/Simulation/ObjectHandlers.cs` 102; the file is 265 lines) accepts the POST. `Scene.IncomingCreateObject` (3083–3124) places the object. Persist once P2.9a exists, as the row already says. `SupportsHeldCrossing` stays false, so `Scene.CrossingHold.cs` stays off. Evidence: the reverse of P2.10b against the same capture, and `object cross: pass` both ways. About a day. No split.

**P2.11c.** `TODO`. Child-agent throttles. Depends on P2.11b, P2.1c, and R26. P2.1c and R26 are `DONE`. Q85 waits on Q80 and QR26. The gate waits for it. It is the scale on the messages section 1's border check already compares. A child's `UpdateChildAgent(AgentPosition)` (`ScenePresence.cs` 5141–5187) scales throttles by `1 - 0.3 * (x² + y²) / 256²`, floored at 0.2, through `LLClientView.SetChildAgentThrottle` (12853) and `LLUDPClient.SetThrottles` (370). An arriving `AgentData` applies throttles at factor 1 (`CopyFrom`, 5332–5333). Outgoing child updates carry `GetThrottlesPacked(1)` (`ScenePresence.cs` 4819 and 5214, `LLUDPClient.cs` 460). R26 owns the rest of the outgoing `AgentData` fields. About half a day. No split.

**P2.12.** `TODO`. The gate report. Depends on P2.1a through P2.11c except P2.5d, plus P2.0e2, P2.0e3, and P2.3d. Q81 is `READY after Q40 to Q80 and Q82 to Q85`. Q86 is inside the TASKS range and outside that queue range. Writing the checklist and the [MANUAL_CHECKS.md](../MANUAL_CHECKS.md) section is about a day once the rows that range keeps are `DONE`. It cannot start earlier. The gate waits for it. It is the last row before G3. The parents this note used to list as a status close (Q65, Q65c, Q65c3, Q65d, Q60, Q60c) are `DONE`. What still holds Q81 is the unfinished rows inside its range. Q40, Q46, Q53 and Q55 are still `SPLIT` in the queue while their task rows are `DONE`. Rule 1 still requires each named `after` item to be `DONE`. That rule is the manager's, and it is not an open question. This note does not change the queue. P2.12 also closes the stale Ferrobot local-chat line in [LIVE.md](../LIVE.md).

## 3. Gaps

One gate check has no P2 row.

**Suggested row P2.0e4, full OAR load, standing at terrain height only.** The manager assigns the ids and the queue rows. PLAN's Phase 2 sentence says "loading OAR files with terrain-height standing only". That is a full OAR load, the same work as Pin8's `load oar`, with the avatar standing on terrain height only. D38's stand-in walks through prims, so standing on a loaded prim is not a Phase 2 check. Saving an archive, and IAR round trips, stay in Phase 5.

Pin8 loads an archive from `ArchiverModule.HandleLoadOarConsoleCommand` (`Source/OpenSim.Region.CoreModules/World/Archiver/ArchiverModule.cs` 95). Without `--merge`, `load oar` still loads region settings when `m_merge` is false (`ArchiveReadRequest.cs` 503–506), loads parcels (508–511), uploads assets unless `--skip-assets` (478–493; the option is `ArchiverModule.cs` 119), and runs `DeleteAllSceneObjects` (553–557). `--no-objects` skips object paths (475–477) and does not skip settings, parcels, assets, or that delete. This row is the full load. The terrain-only flag set `--merge --merge-terrain --no-objects --skip-assets` is a different command, and it is not this row.

Depends on P2.0e, which is `DONE`. The heightmap writer from P2.0e2 is the store side of terrain. The object loaders are already `DONE`. Each part is about a day, and the evidence for every part is a Pin8 `load oar` of the same archive. The manager assigns the ids. This file does not add them to TASKS or the queue.

1. **Control file and region path** (about one day). `FindAndLoadControlFile` (`ArchiveReadRequest.cs` 616), `LoadControlFile` (1300), and the region path map (461). The loaded region follows the control file and the path the way Pin8 does.
2. **RAW32 loader** (about one day, beside part 1). `World/Terrain/FileLoaders/RAW32.cs` (205 lines). A RAW32 terrain file in the archive decodes to the same samples Pin8 decodes.
3. **Terrain merge** (about one day, after part 2). `TerrainModule.LoadFromStream` merges through `MergeWithBounding` (`TerrainModule.cs` 480–493). `LoadTerrain` (`ArchiveReadRequest.cs` 1271) calls `LoadFromStream` at 1284 for a plain load. Terrain paths are 498–501. The merged heightmap matches Pin8.
4. **Deferred terrain save** (about one day, after part 3). The deferred `SaveTerrain` on the terrain tick (`TerrainModule.cs` 948–960). After the tick, the stored terrain row matches the loaded map.
5. **Objects** (about one day). Object paths are read when `m_noObjects` is false (475–477). A non-merge load runs `DeleteAllSceneObjects` first (553–557). The objects Pin8 loads are present. The avatar still stands at terrain height and walks through them.
6. **Parcels** (about one day, beside part 5). Land data at 508–511. The loaded parcels match Pin8. Phase 2 still writes only the default parcel on the ordinary path (D39). This part is the archive load.
7. **Region settings** (about one day, beside part 5). Settings at 503–506, when the load is not a merge. The loaded settings match Pin8.
8. **Assets** (about one day, beside part 5). Upload unless `--skip-assets` (478–493). The uploaded assets match Pin8's upload of the same archive.

The gate waits for these parts under the PLAN sentence. Under G3's current one-line list, the person never loads an archive. Question 1 decides which reading the gate uses.

Considered, and not proposed as rows:

- Permissions. P2.7a, P2.7b, and P2.7c are `DONE`. Edit, take, and delete call them.
- Wind and cloud `LayerData`. Terrain `LayerData` is P2.0e. The gate does not ask for wind or cloud.
- Worn attachments. P2.0f sent no attachment block. `Client_OnAvatarNowWearing` is `AvatarFactoryModule.cs` 1311 and `MergeNowWearing` is 1370. Question 8 asks whether the appearance check includes a worn attachment. Attachment rez stays out of Phase 2 either way.
- The draw-distance ini values. P2.5c is the interest list. Stored objects already arrive as full updates through the done P2.5a leaves. A separate ini follow-up is not required to name a new row.
- Mesh upload that builds a scene object. It is not one of the checks in section 1.
- Walking across the border. D38 defers it. Section 1's border check is the child-agent view plus the object crossing.
- `RenderMaterials`. P2.5d already exists and D39 leaves it past the gate.

## 4. Critical path and three coders

The long pole is the object spine. Q65 is `DONE`, so the spine starts today. In series it is P2.5c's properties part, then P2.8a, then P2.8b1, then P2.8b2's two parts, then P2.9a (two parts), then P2.9b (two parts): about nine days. Culling runs beside properties and does not add a day, because P2.8a starts after properties. Take (one day) and rez (two days) branch off P2.8a. Delete-to-trash is one more day after P2.9a. Object cross out is two days from P2.8b1. That series is longer than bakes (about five days if P2.3c1's three parts run in order, shorter when the upload runs beside wearables, then P2.3c2's two parts) and longer than children (about four and a half days: Q79c2a, Q79c2b, P2.11b's two parts, and P2.11c).

One lane takes P2.5c today. Motion and parcels are unblocked, and they sit behind bakes or children so the spine stays occupied.

Dependency order of the rows the gate waits for:

1. No status close is left on the spine. P2.5a / Q65 is `DONE`, so P2.5b and P2.5c can start. P2.5a3, P2.5a3c and P2.5a4 are `DONE` and gated only P2.12's range. P2.6a and P2.6a3 are `DONE`, so P2.9a's task dependency and P2.6b's task dependency are satisfied. P2.0d5's cell can be corrected in the same bookkeeping pass. It does not unblock the spine.
2. In parallel from today:
   - Object spine: P2.5c properties, then P2.8a, then P2.8b1, then P2.8b2's two parts. From P2.8b2 part 2, link (P2.8c) and SQLite persistence (P2.9a, then P2.9b). P2.5c culling and P2.5b run beside properties and do not gate the spine. From P2.8a, take (P2.8d part 1, then P2.8e). Delete-to-trash (P2.8d part 2) waits on P2.9a. From P2.8b1, object cross out (P2.10b, then P2.10c).
   - Appearance: P2.3c1, then P2.3c2. Both can start today. This chain is not the long pole.
   - Children: Q79c2a, then Q79c2b, which finishes P2.11a, then P2.11b, then P2.11c. Q79c2a can start today. After Q79c2b, the section 1 teleport re-run is part of that evidence.
   - Motion (P2.3b1, then P2.3b2) and parcels (Q61b, then Q61c) can start, because their dependencies are `DONE`. They sit behind bakes or children. Q61b does not need another parent closed: P2.6a3 is `DONE` and Q61a is `DONE`.
   - OAR, the suggested P2.0e4 parts, any time. They depend only on P2.0e, plus the terrain order in section 3. They are eight parts of about a day, and several of them can run beside each other.
3. P2.12, after the unfinished rows in the range it names are `DONE`. Q81's queue range is Q40 to Q80 and Q82 to Q85. Q86 is outside that queue range and inside the TASKS range. Questions 4, 5, 6, 7 and 8 are what that range keeps. Then G3.

Three coders, from today:

| Coder | Start now | Then |
| --- | --- | --- |
| 1. Object spine | P2.5c properties, then stay on the spine: P2.8a, P2.8b1, P2.8b2's two parts, P2.9a, P2.9b. P2.5c culling and P2.5b fill gaps beside properties. | Take (P2.8d part 1) and rez (P2.8e) once P2.8a is `DONE`, or hand them to a free coder. Object cross out (P2.10b, then P2.10c) once P2.8b1 is `DONE`. Delete-to-trash after P2.9a. |
| 2. Appearance | P2.3c1 (wearables, then cached textures, with the upload beside wearables), then P2.3c2 (two parts) | The first spine row that is free, or the next P2.0e4 part. Motion if coder 3 is still in children. |
| 3. Children | Q79c2a, then Q79c2b, then P2.11b (two parts), then P2.11c, including the neighbour-teleport re-run with the child open | Parcels (Q61b, then Q61c), then motion (P2.3b1, P2.3b2) if coder 2 has not taken it. Object cross in (P2.10c) once P2.10b is `DONE`, if coder 1 has not taken it. |

Whoever is free also picks up the next P2.0e4 part. Those parts wait on P2.0e, which is `DONE`, and on the terrain order in section 3.

These can run beside the three lanes and are not predecessors of them:

- P2.10a2, the seated two-C# capture, any time.
- P2.6a3a2, the Local groups connector, any time. It does not unblock the Robust-grid checks.
- P2.8c part 2 (duplicate) beside P2.9a, once P2.8b2 has landed and part 1 of P2.8c has the link path.
- P2.9b's MySQL and PostgreSQL parts, in order, as soon as P2.9a part 1 has proved the writer. The gate still waits for both.
- The P2.0e4 parts that do not wait on each other: the control file, the RAW32 loader, objects, parcels, settings and assets.

P2.5d stays out of this schedule.

Coder 1 is on P2.5c from today. Motion and parcels stay off that lane. A coder who finishes bakes or children takes the next free spine row before starting motion or parcels.

## 5. Questions

Questions 2, 3 and 9 are answered. Questions 1, 4, 5, 6, 7 and 8 stay open.

1. Is G3's one-sentence list (log in, walk, see another avatar, build, chat, teleport to and from the C# region, Firestorm and Aurora) the whole gate, or does the person also do the PLAN items that sentence leaves out (stored objects, inventory take and rez, appearance, the neighbour in view, an object crossing, the store swap, and a full OAR load with the avatar standing at terrain height only)? If the sentence is the whole gate, P2.12's checklist is that sentence and the OAR parts are PLAN scope that G3 does not exercise. If the PLAN sentence is the gate, section 1 is the script P2.12 writes into [MANUAL_CHECKS.md](../MANUAL_CHECKS.md).

2. Answered by [D38](../DECISIONS.md). "Cross a border" is teleport both ways plus child agents, so the C# neighbour is in view, together with a riderless object crossing (`SupportsHeldCrossing` false). Walking across the border waits. P2.12 already names object cross. This is not an open question.

3. Answered by the G3 row. The person's check is Firestorm and Aurora. The script in section 1 is the same for both. The G3 evidence names both viewers. This is not an open question.

4. Does the Robust-grid gate require the Local groups connector (P2.6a3a2 / Q86)? The harness grid uses Robust, and Q60c3 already delivers the overlay from remote memberships. Q86 is inside P2.12's TASKS range (P2.1a through P2.11c) and outside Q81's queue range (Q40 to Q80 and Q82 to Q85). Q86 is `READY` now. Leaving it out of the gate lets the three coders skip it. Keeping it in the TASKS range holds P2.12. The question is which of those two ranges the gate uses.

5. Does G3 wait on the seated two-C# fixture (P2.10a2 / Q76b)? [D39](../DECISIONS.md) says riders and script state wait for Phases 3 and 4, which already points to dropping P2.10a2 from the gate range. Held crossing is off. P2.10b's evidence is the riderless capture. P2.10a2 is inside the written P2.12 TASKS range and can be captured in parallel. Dropping it from the range lets P2.12 ignore it.

6. Does take include `DeRezAction.Return` and `GodTakeCopy`, or only `Take`, `TakeCopy`, and `Delete`? The proposed P2.8d split covers the three the row names. Return is the land path, and Phase 2 writes only the default parcel.

7. Does rez include a coalesced object and a rez from a prim's inventory, or only one user-inventory object onto the terrain? The proposed P2.8e split does the single object and the coalesced path. The prim-inventory path (`fromTaskID` non-zero) waits on this answer.

8. Is a worn attachment part of the appearance check, or is the check the viewer-side bake only (P2.3c1 and P2.3c2)? P2.0f sent no attachment block. `Client_OnAvatarNowWearing` exists. Attachment rez is not in either reading of this question.

9. Answered. Whether a finished split parent must be marked `DONE` before the next item starts is the manager's queue rule, not a maintainer decision. Rule 1 still requires each named `after` item to be `DONE`. The parents this note used to wait on are already `DONE`: P2.5a / Q65, P2.5a3, P2.5a3c, P2.5a4, P2.6a and P2.6a3. The spine does not wait on a status close. Q40, Q46, Q53 and Q55 remain `SPLIT` in the queue with their task rows `DONE`, and Q81's `after` list still names that range. This note does not change the queue. This is not an open question.
