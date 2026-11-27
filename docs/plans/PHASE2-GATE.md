# Phase 2 gate map

Planning note, 26 November 2026. The evening revision on that day writes the assigned rows into [TASKS.md](../../TASKS.md) and [WORK_QUEUE.md](../../WORK_QUEUE.md). The morning map proposed splits and left them unassigned. The rows the manager has since assigned are Q383 to Q387. Citations were rechecked at Pin9 `41f18f983bb2bfc82e033fed410d361ba57d5aa4`. That tree differs from Pin8 `c401bceecd1684832ae8ba86a31b5ee01a4ded7b` only in Jolt, so a Pin8 capture of a packet, archive or database path remains evidence. An independent review of this map, read against origin/main `9c6259fb32ff5d4807dbcca5ead779ccc552cb71`, is worked in below. No viewer, store swap or live scenario was run for that review.

Standing decisions: [D38](../DECISIONS.md) (the walking stand-in, next door as teleport both ways plus child agents, viewer-side bakes, all three region stores) and [D39](../DECISIONS.md) (scene state on the frame thread, no held object crossing, the ordinary path writes only the default parcel, persistence timing, materials passed through). D39 was amended on 26 November 2026: a Phase 2 OAR load persists the parcels, restored objects, and region and estate settings it loads, as the pin does. Starting scripts after a load stays out until Phase 4. Saving an archive, and IAR round trips, stay in Phase 5. The maintainer may override D39.

The queue's rule 10 still records two coders. The four lanes in the evening section are the schedule for the remaining gate work when four people are available.

## 1. The gate

The gate is the scope [PLAN.md](../PLAN.md) and D38 and D39 already define. It includes all three region stores, a durable OAR restoration that keeps non-default parcels, both teleport directions with child visibility, a riderless object crossing, and viewer-side bakes. The avatar stands at terrain height. Walking across the border, parcel editing, script execution, server baking, the materials capability, and archive round trips beyond this load stay in later phases.

G3 is a separate milestone after P2.12. G3, in [TASKS.md](../../TASKS.md), is the maintainer logging in with Firestorm and Aurora, walking, seeing another avatar, building, chatting, and teleporting to and from the C# region next door, following a Phase 2 section in [MANUAL_CHECKS.md](../MANUAL_CHECKS.md). That section does not exist yet. P2.12 writes it, from the automated evidence below. Passing G3 on its own is the human smoke test. It does not replace the automated gate. Question 1 is closed.

Automated evidence is a bot scenario, a capture compare, or a live run in [LIVE.md](../LIVE.md). [TESTING.md](../TESTING.md) keeps the harness as protocol evidence. A passing run is the protocol result. The person records viewer versions in the G3 row. Ferroan has no physics switch: it always runs the walking stand-in. A C# walk, animation, or crossing capture that the compare uses is taken with `--physics basicphysics`.

`cargo xtask live run all` runs the names in the bot's scenario list. Today those pass lines include `login: pass`, `walk: pass (forward movement, observer kills=1, leaving viewer kills=0)`, `chat: pass`, `build: pass`, `teleport: pass` with the landing position, `texture: pass`, and `inventory: pass`. `cargo xtask live compare region` prints `region: match` when both a C# home and a Ferroan home print `login: pass`, `chat: pass`, `teleport: pass`, `teleport neighbour: pass (1001,1000 and back)`, and `live: run passed`. The region comparison recognises those login, chat, teleport and completion lines. At this tip the aggregate and build registrations refuse a Ferroan target (`tools/bot/src/register.rs` 439–446 and 489). Q385 makes those registrations accept a Ferroan target and is the manifest P2.12 consumes. Dedicated probes that `all` leaves out are named in the table.

| What the person does | Automated evidence that must already pass |
| --- | --- |
| Log in through C# Robust and arrive standing on the Ferroan region | Already passing: `cargo xtask live run login grid --region ferroan` prints `login: pass`, and `compare region` includes it. Circuit, seed, event queue, arrival, and logout are P2.0a through P2.0d4 and P2.0d6. `cargo xtask live run caps grid --region ferroan` covers the seed and the held empty poll. |
| See the region and its terrain, standing at terrain height | Already passing: P2.0e `LayerData` matches the stored heightmap and `AgentMovementComplete` puts the avatar at that height. P2.0e2 and P2.0e3 write and load the default terrain on SQLite, MySQL, and PostgreSQL. The walk line lands at that height. |
| See objects stored in the region | Full updates of a handed-off store already pass: `cargo xtask live run store-handoff grid --physics basicphysics` and `cargo xtask live run prim-load grid --physics basicphysics` (P2.5a4a and P2.5a4b). Properties and kills are P2.5c1, done (Q208). Before this Firestorm check, P2.5b1 must match the zero-coded compressed update against a Pin9 capture taken with `SupportViewerObjectsCache` on, the default (`LLUDPServer.cs` 361). The routine at `LLClientView.cs` 7892 sits inside the block comment that runs from 7891 to 8132. Both live calls use `CreateCompressedUpdateBlockZC` at 8134 (the calls are at 5472 and 5510), including the packet-boundary case. P2.5b2 must match the arrival probes. With culling off, the default, that decision is `ScenePresence.SendInitialData`. Distance culling stays off unless the capture turns it on, and that capture is P2.5c2. Q387 ties cache eligibility, selection and the stored pseudo-CRC together. |
| Walk | `walk: pass` already, from the stand-in (P2.2b1, P2.2b2). P2.3b1 and P2.3b2 are done (Q49, Q50): walk then stand with the same animation UUIDs and sequence numbers as a basicphysics C# region. Record terrain-standing movement against that BasicPhysics baseline. |
| See another avatar | Already passing: `avatars: pass` (P2.3a) and `UUIDNameReply` (P2.3d), and the animation packets from Q50. Before the person looks, P2.3c2's observer must receive the same `AvatarAppearance` texture entry C# sends. Without P2.3c2 the other avatar stays grey or Ruth. Q386 adds the SeeAVs-false parcel, the edit and take-copy denial, and the already-open child. |
| Chat | Already passing: `chat: pass` (P2.0g), including the second account on channel 0 and DEBUG_CHANNEL type 6. P2.0g is done. The Ferrobot table in [LIVE.md](../LIVE.md) still lists Ferroan local chat as pending, because `xtask/src/ferrobot.rs` 447–449 prints that line for a Ferroan region. The companion's chat runs only on C# (457). Q228 (P2.0g2) enables the exchange and records a real passing line. [LIVE.md](../LIVE.md) changes only after that run. |
| Edit and rez an object | P2.8a through P2.8c must pass first: rez, select, move, rotate, texture, link, and unlink, with `build: pass` on the Ferroan region and a new `link: pass`. The present build sequence checks a matching scale, a rename, a move and scale, then a kill. Rez from inventory is P2.8e. Permissions for rez, edit, move, delete, take, link, and copy are P2.7a through P2.7c, already done, and these rows call them. Q386 checks a non-owner denied an edit and a take-copy beside the owner allowed one. |
| Build, restart, and read the same store from C# | P2.9a is the SQLite store swap: build on Ferroan, stop, start Ferroan again and see the build, then start C# on the same file and read the same objects. Periodic backup and the shutdown forced save are proved separately. P2.9b repeats the swap on MySQL and on PostgreSQL. P2.9c (Q383) is Ferroan reading its own store back on MySQL and PostgreSQL: the parcel map, region settings and environment, and the estate. SQLite already has those reads. D38 puts all three stores on the gate. |
| Use inventory | The window and folder fetch already pass: `inventory: pass` and the fetch caps (P2.0d7). Take and delete use different permission checks (`Scene.Inventory.cs` 2935–2966 and 2997–3019). The admitted trash path shares one deleter (`AsyncSceneObjectGroupDeleter.cs` 128–135). Rez of a taken item is P2.8e and waits on the take part. Single then coalesced rez, no-copy consumption, and next-owner masks are inside Q73. A C#-taken asset is rezzed in Ferroan, and a Ferroan-taken asset is rezzed in C#. |
| Change appearance | P2.0f already sends the unbaked `AvatarAppearance` and avatar `ObjectUpdate`. P2.3c1 matches wearables, the cached-texture query, and the two-stage `UploadBakedTexture` replies with C# for the same account. The upload can proceed beside wearables. The upload compare is the two LLSD replies, the remote-address check, and the cached local asset. P2.3c2 stores through `setavatar` and broadcasts the baked texture entry, including another circuit's texture entry after relog. `ServerSideBakingModule` stays out (D38). Worn attachments are outside this check. |
| Teleport to a C# region and back | The hop without a child already passes: `teleport neighbour: pass (1001,1000 and back)` and `cargo xtask live run hop grid --region ferroan` / `neighbour grid --region ferroan` (P2.0i). Q386 owns the already-open-child rerun. At Pin9 an in-range hop takes the child seed and substitutes a fresh path when none exists (`EntityTransferModule.cs` 1045–1061). Negotiation selects V2 when the outbound version is at least 0.2 (1064–1068; also 1395–1418 and 1461–1479). Lines 1134–1142 and 1292 belong to the older transfer branch. The proof asserts seed reuse against an observed, established child and names the negotiated path. Q80b is the `AgentPosition` mirror and does not own this rerun. |
| Cross a border | Read as D38's next-door pair plus a riderless object crossing. The neighbour is in view while the avatar stays root at home: P2.11a is done, including `cargo xtask live run child-view grid --physics basicphysics` with Ferroan as home (Q79c2b). P2.11b1 accepts the C# neighbour's child (Q80a). P2.11b2 (Q80b) and P2.11c (Q85) remain. An object crosses both ways with source deletion only after a successful `CreateObject` (`EntityTransferModule.cs` 2978–2988). A refusal keeps the source. Walking across the border stays deferred (D38). |
| Stand on an OAR-loaded region | The P2.0e4 family, then Q384. A full `load oar` of the same archive: settings, parcels, terrain, objects and assets, replace and merge, in Pin9's order. The avatar stands at that terrain height and walks through the loaded prims, because the stand-in does. About Land on both sides of a border, the telehub prim, and a restart on each database are part of Q384. |

P2.12 gathers those lines from the leaf checklist, adds the Phase 2 section to [MANUAL_CHECKS.md](../MANUAL_CHECKS.md), and stops. It consumes Q385. G3 is the person, after that section exists. A Ferrobot session that watches another avatar walk and build is part of P2.12's evidence, as far as the pinned Ferrobot API allows. Region IM stays out. The Ferrobot local-chat line changes only after Q228 has a real passing run.

## 2. Landed work, and the leaves still open

These leaves landed on origin/main after the morning map and are not charged again: Q49 and Q50 (#351, #353), parcel properties Q61c1 and Q61c2 (#345, #348), home-driven child visibility Q79c2b (#356), incoming child admission Q80a (#366), properties and kills Q208 (#363), Q215 RAW32 (#357), Q216a merge geometry (#362), Q218 owner transformation (#355), Q220 in-memory parcels (#359), Q226 environment storage (#361), and Q86, the Local groups connector. Q68 waits only on Q208 and Q64, both done. P2.3b1 and P2.3b2 are done. P2.0d5's stale cell has no bearing on this path.

Closed roll-ups, because every leaf is done on this main:

| Parent | Leaves | Status now |
| --- | --- | --- |
| Q61 / P2.6b, and Q61c / P2.6b3 | Q61a, Q61b, Q61c1, Q61c2 | Done. Parcel editing stays in Phase 5. |
| Q79 / P2.11a, Q79c, and Q79c2 | Q79a, Q79b, Q79c1, Q79c2a, Q79c2b | Done. Walking across the border stays deferred. |

P2.11b stays open because Q80b is still to do. Q85 waits on Q80b. Split parents are not counted again beside their leaves: Q66 (Q206, Q207), Q67 (Q208 done, Q209 open), Q70 (Q210, Q211), Q72 (Q212, Q213), and Q216 (Q216a done, Q216b open). Q214 stays ready, and its task P2.0e4.1 stays to do. Its feature head is outside this main's ancestry, so it is counted once. Q379 and Q380 stay ready after Q50 on this main.

Q81's leaf checklist is every remaining gate leaf, once. It is written on the P2.12 row. Q76b is reference-only and outside G3, so it is not on that list. Q382 is on Q217's unmerged branch and is on the list.

### Off the gate

**P2.5d.** To do. Not queued. The `RenderMaterials` cap. D39 keeps Phase 2 to material ids in the texture entry and extra params. Critical path: no.

**P2.10a2 / Q76b.** Reference-only, outside G3. D39 defers rider transfer. P2.10b's evidence is the riderless capture. The row can be captured beside the gate. It does not hold P2.12.

**Task-inventory rez, `DeRezAction.Return`, and `GodTakeCopy`.** Separate entry points. A passing take, delete, or user-inventory rez does not cover them. No new row is assigned for them.

**Worn attachments.** The appearance check is the viewer-side bake (P2.3c1 and P2.3c2) seen by a second viewer. Attachment rez stays out of Phase 2.

### Splits that stay inside an existing id

Q51 (P2.3c1), about three days, in three parts of about a day. Wearables (`AvatarFactoryModule.cs` `Client_OnRequestWearables` 1275). The cached-texture query (`Client_OnCachedTextureRequest` 1437 and `ValidateBakedTextureCache` 549). The two-stage baked upload (`UploadBakedTextureModule.cs` 113–148 and `BakedTextureUploader.process` 189–220). The upload can proceed beside wearables. `SaveBakedTextures` (`AvatarFactoryModule.cs` 315) is the OSSL path and is not this upload. Compare the replies for the same account, the allowed uploader address, the uploaded bytes, and the cached local asset. Q52 is about two days after Q51: store and the observer broadcast, including the texture entry after relog.

Q71 (P2.8c), about two days after Q211. Link and unlink, then duplicate. Compare owner and observer identities, local parents, and relative and world transforms. The duplicate is a distinct group and the source stays unchanged.

Q73 (P2.8e), about two days after Q212. Single rez, then coalesced rez, consuming a no-copy item and applying next-owner masks. Rez a C#-taken asset in Ferroan and a Ferroan-taken asset in C#.

Q74 (P2.9a), about two days after Q211. It may split into a forced save and a periodic backup. Until that split is assigned, both proofs stay on this id. The acceptance is below, under the citation corrections.

Q383 may split per backend, and Q384 may split into the replace path and the merge path, if either proof exceeds about a day. The parent acceptance stays the whole proof.

## 3. Rows added by this revision

Each new row is about a day, with C# at Pin9, and evidence against that C# rather than a Ferroan-only round trip.

**P2.9c / Q383.** Ready now. Ferroan reads its own stores back on MySQL and PostgreSQL. SQLite already reads them. The parcel map is not built for those backends (`crates/ferroan-region/src/main.rs` 535–554). Region settings stay on the fresh handshake defaults (`main.rs` 483–492, `crates/ferroan-region/src/settings.rs` 6–48). The estate read does not open MySQL or PostgreSQL (`crates/ferroan-scene/src/estate.rs` 8–12). The scene crate can load an environment row on all three databases, and the region process does not apply that row at startup. Evidence: Pin9 writes a store with non-default values, and Ferroan reopens it on each backend. Default and non-default values tell saved data from fallbacks. Split per backend if one day is not enough.

**P2.0e4.9 / Q384.** Ready after every P2.0e4 part (including Q382 once that row is on main), the four writers, P2.9a, P2.9b, and P2.9c. The integrated console `load oar`, replace and merge, in Pin9's order (`ArchiveReadRequest.cs` 475–511, 549–565, 764–781, 1011–1028, 1248–1257). Clear old objects on replace. Install parcels before objects. Remap a telehub to the new object id. Save settings. Admit restored objects for backup (`AddRestoredSceneObject`, and `SceneObjectGroup.cs` 3234–3256). Evidence: a real console load against Pin9 on SQLite, MySQL, and PostgreSQL, with pre-existing geometry and land, a linked object with task inventory, a telehub, and a referenced asset. Compare through a one-to-one id map. Restart with each implementation. About Land on both sides of a border. The telehub points at the right prim. A nonzero object Category is included here, or the accepted difference is recorded (see the evening risks). Split into replace and merge if one day is not enough.

**P2.12a / Q385.** Ready now. The gate runner. One executable manifest of every required scenario, each with its Pin9 C# baseline, its Ferroan result, and pass, skip, or fail. It fails when a required scenario is absent. The aggregate and build registrations accept a Ferroan target (`tools/bot/src/register.rs` 439–446 and 489). Effects are captured at the second viewer or service. Q81 consumes this row. Writing the report is still P2.12.

**P2.3g / Q386.** Ready after Q379, Q69, Q211, and Q80b. Viewer-relationship checks, one row.

- A SeeAVs-false parcel seen by a viewer outside it, a same-parcel viewer, and the god exception, through a border transition: hide, reappear, and appearance and animation suppression.
- A non-owner denied an edit and a take-copy, beside the owner allowed one (`Scene.Inventory.cs` 2935–2948 and 2997–3013).
- The neighbour child is already open before the teleport. The home viewer sees the neighbour's prim and avatar on its child circuit. The old root transitions. Both directions. This is the already-open-child teleport rerun. Q80b does not own it.

**P2.5b3 / Q387.** Ready after Q68, Q206, Q207, and Q74. The shared cache, selection, and persistence test. Freshly rezzed and selected. Deselected but recently edited. Just before, at, and after the five-minute eligibility. Changed again. Physical, temporary, and attached groups excluded (`SceneObjectGroup.cs` 199–219). Two viewers and the stored pseudo-CRC are observed (`LLClientView.cs` 5569, `SQLiteSimulationData.cs` 1877 and 2308). The age is driven deterministically. Pin9 packets back the reachable states. About a day.

### Citation corrections

These replace the morning map's readings. The same corrections are on the task rows.

1. **Compressed encoder** (P2.5b1 / Q206). `CreateCompressedUpdateBlock` at `LLClientView.cs` 7892 is inside the block comment from 7891 to 8132. The live implementation is `CreateCompressedUpdateBlockZC` at 8134, called at 5472 and 5510. The capture is that on-wire form, including a block that crosses a packet boundary.
2. **Physical flag** (P2.8b22 / Q211, with Q206 and Q207). Dynamics wait for Phase 3. The stored flag already changes what a viewer receives. `SceneObjectGroup.UsesPhysics` (`SceneObjectGroup.cs` 510–514) is the root flag, and `IsViewerCachable` (199–208) excludes those groups from cached and compressed eligibility. Q211 toggles the flag. Q206, Q207, and Q387 observe the eligibility change.
3. **`UpdatePermissions`.** The method at 4823 is `SceneObjectPart.UpdatePermissions` (`SceneObjectPart.cs` 4823–4882), after `SceneGraph.UpdatePrimFlags`. The group method is `SceneObjectGroup.UpdatePermissions` (`SceneObjectGroup.cs` 5445).
4. **Take and delete** (P2.8d1, P2.8d2, P2.8e). The branches use different permission checks. `TakeCopy` requires `CanTakeCopyObject` and a denial can send an alert (2997–3013). `Take` requires `CanTakeObject`. `Delete` requires `CanDeleteObject`. Delete can remove without trash. The god or other-owner case matters (`Scene.Inventory.cs` 2935–2966). The common admitted trash path uses the same deleter (`AsyncSceneObjectGroupDeleter.cs` 128–135).
5. **Two arrival calls.** `ScenePresence.cs` 2568–2577 runs `EnableChildAgents` only under the crossing-update flag. Ordinary initial-data work is `SendInitialData` at 4395–4407. P2.11a3b2 already distinguishes them.
6. **In-view teleport seed.** Owned by Q386. `EntityTransferModule.cs` 1045–1061 takes the child seed only on the in-range branch and substitutes a fresh path if none exists. Assert seed reuse against an observed, established child, and identify the negotiated path. V2 is selected at 1064–1068.
7. **Source deletion** (P2.10b / Q77 and P2.10c / Q78). `CrossPrimGroupIntoNewRegion` deletes the source at `EntityTransferModule.cs` 2988 only after `CreateObject` succeeds at 2978–2984. A refusal keeps the source, for the source viewer and for the store. After Q74 and Q75, reopen both stores. A deleted group must not come back, and a crossed group must not be lost.
8. **Asset existence** (P2.0e4.8 / Q223). With forced cache and an available cache, `ArchiveReadRequest.cs` 367–380 uploads directly. The ordinary path handles a null or wrong-sized existence reply before individual uploads at 383–408. The evidence states which mode was captured.
9. **Backup timing** (P2.9a / Q74). Two hundred frames is the default of `m_update_backup` (`Scene.cs` 385). The region reads `UpdateStorageEveryNFrames` (1150) and runs the backup when periodic backup is enabled and the frame matches (1879). `SceneObjectGroup` tests last change or first change (241–259, 302–313) and uses different thresholds in an empty region. Shutdown's forced save is `Backup(true)` at `Scene.cs` 1615, proved separately from the periodic backup. The durability order is: an edit, then a periodic backup; a later edit, then a forced shutdown; a pending edit, then a take, delete, or cross, then a backup, then a restart. The linked child's inventory holds a distinguishable item. A deleted group must not come back.
10. **Archive control lines** (P2.0e4.1 / Q214). The region-path lookup is `ArchiveReadRequest.cs` 460. Line 461 skips a destination the control file did not select. `LoadControlFile` begins at 1298. Line 1300 instantiates the namespace manager. `FindAndLoadControlFile` at 616 is the right call.
11. **Child throttle** (P2.11c / Q85). Before squaring, `ScenePresence.cs` 5170–5187 subtracts the respective region size once, and only when that coordinate is positive and above that dimension. The denominator is the fixed `Constants.RegionSize`. The floor is 0.2. The proof uses a nonstandard-sized neighbour and signed coordinates. The factor-one full update is at 5333.

### Verified entry points

Rechecked at Pin9. Existing Pin8 captures of these paths remain evidence.

| Subject | Entry points |
| --- | --- |
| Walking, animations, appearance | `ScenePresenceAnimator.cs` 187, 260–570, 576–594, 838. `LLClientView.cs` 4666, 9466. `AvatarFactoryModule.cs` 1275, 1437, 549, 1296, 884, 274. `UploadBakedTextureModule.cs` 113, 189. |
| Cache, interest, properties | `LLClientView.cs` 5020, 5206–5207, 5295, 4989, 5532, 5569, 8580, 9744, 5857, 5947–5964, 6173, 6203, 6222, 6396, 6431, 2374. `SQLiteSimulationData.cs` 1877, 2308. `ScenePresence.cs` 4361–4384. |
| Editing and inventory | `LLClientView.cs` 9659, 9755, 10001, 10012, 12623, 9674, 9711, 9861, 10128, 10142, 9818, 10023, 9624, 9642, 9726. `SceneObjectGroup.cs` 4199, 4489. `Scene.Inventory.cs` 3138, 3269. `InventoryAccessModule.cs` 442, 892, 902, 933, 944. |
| Persistence and archive | `SQLiteSimulationData.cs` 534, 557, 2132, 2549, 2598, 2632, 974, 1018, 367, 440. `MySQLSimulationData.cs` 130, 260, 692, 712, 1967. `PGSQLSimulationData.cs` 259, 448, 483, 758, 808. `ArchiveReadRequest.cs` 616, 764–781, 884, 973, 991, 998, 1012, 1028, 1184, 1202, 1251, 1253. `ArchiverModule.cs` 255. `TerrainModule.cs` 480–493, 948–960. |
| Neighbours and crossings | `Scene.cs` 4117, 4211, 4773, 4819, 4848, 4860, 3083–3124. `ScenePresence.cs` 4819, 5141–5187, 5214, 5333. `SceneObjectGroup.cs` 883–902, 960. `EntityTransferModule.cs` 2968–2988. `ObjectHandlers.cs` 102. |
| Parcel requests | `LLClientView.cs` 10874 handles the request. `SendLandProperties` is 6757. Archive land installation (`ArchiveReadRequest.cs` 1011–1028) and the land-store subscriptions (`Scene.cs` 952–953) are separate from those messages. Q61's leaves already deliver the messages. |

## 4. Questions settled on 26 November 2026

1. **Closed.** The gate is the PLAN, D38, and D39 scope in section 1. G3 is the human check after P2.12. P2.12 writes section 1 into [MANUAL_CHECKS.md](../MANUAL_CHECKS.md).
4. **Closed.** Q86 is done. The Robust-grid gate does not wait on another Local groups row. Q60c3 already delivers the overlay from remote memberships.
5. **Closed.** Q76b is reference-only and outside G3.
6. **Closed as a boundary.** Take, TakeCopy, and Delete are on the gate, with the permission differences in section 3. Return and GodTakeCopy are not implied and have no new row.
7. **Closed as a boundary.** Q73 covers one user-inventory object and the coalesced path, including no-copy consumption and next-owner masks. Rez from a prim's inventory (`fromTaskID` non-zero) is not implied and has no new row.
8. **Closed as a boundary.** The appearance check is the viewer-side bake. A worn attachment is not part of it.

G2 and G3 stay for the maintainer to record. A short human rehearsal can sample About Land, a relogged build, and a neighbour already in view. Automation carries the three-database and archive matrix.

## Revision, 26 November 2026 evening

### What changed since the morning

The morning map still had question 1 open, still proposed the OAR parts and the four writers without assigning them, and still scheduled three coders with parcel delivery and movement queued behind child agents. Those parcel leaves, the child-view frame drive, the animation rows, and Q86 have since landed. This revision closes question 1, closes the Q61 and Q79 roll-ups, writes Q383 to Q387 into the task list and the queue, applies the ten citation corrections above, and replaces that three-coder schedule with the four lanes below. Q379 and Q380 remain ready on this main. Q382 remains on Q217's unmerged branch `feat/q217-oar-terrain-save`.

### Critical path

The longest chain is Q68, then Q69, then Q210, then Q211, then Q74, then Q75, then the parcel writers Q224 and Q221, then Q384, then P2.12. Q68 can start now.

Q219 joins that work. It needs Q75, Q225, and Q377, and it also waits on Q74. Q218 is already done. Q222 joins Q225, Q226, and Q227. Q226 is done. The archive can set the finish even when the two object writers land on time. Bringing the land and settings writer fixtures forward, and giving Q75's two backends to different free lanes, shortens the tail. The three-store check still waits for both backends.

### Four lanes

| Lane | First work and handoffs |
| --- | --- |
| 1. Object edits and persistence | Start Q68, then Q69, Q210, Q211, Q74. Own the persistence contract through Q75. Hand Q212 to another lane after Q68, and Q77 after Q69. |
| 2. Viewer updates and inventory | Start Q206, Q207, and Q209. Reuse their baselines when Q68 is integrated. Pick up Q212, then Q73. Q213 after Q74. Q71 after Q211. Link and duplicate stay separate evidence inside Q71. Single and coalesced rez stay separate evidence inside Q73. |
| 3. Archives and stores | Q214, then Q378 and Q216b, alongside Q217, Q377, Q223, and Q227 as their prerequisites allow. Prepare the database reader and writer comparisons early, including Q383. Finish Q225, Q224, and Q219, Q221, and Q222 when their writers are available. Q384 is the integrated load after those parts. |
| 4. Neighbours and avatars | Q80b, then Q85. Q51, then Q52, with Q379, Q380, and Q228. Q386 after Q379, Q69, Q211, and Q80b. Share Q77, then Q78, and one Q75 backend, with the first free lane. Q76b stays outside the critical chain. |

Whoever is free takes the next unblocked row on the critical path before starting a row that can wait. Q385 can start now. P2.5d stays out of this schedule.

### Risks

**Stacked branches.** The queue uses squash merges, and a reviewed local commit precedes publication. Another push restarts the checks. Terrain merge has already stacked on the reader, and the appearance, object, and archive rows share scene, store, and scenario entry points. A passing child branch is evidence for that composite. Record its base and its fixture hashes. When the parent lands, rerun the affected replay and the aggregate scenario registration. Reserve the full two-server and store rehearsal for the binary the gate report names. Give the four lanes independent reference fixtures where they can.

**Cache coupling.** Backup eligibility excludes temporary, in-transit, physical, selected, attached, mesh-flagged, and keyframed groups, and it uses more than five minutes since the last change. That timestamp also participates in backup. Selecting another owner's object can force a full update. A capture of an old stored object and a separate cache-disabled rez test can both pass while the live transitions are wrong. Q387 is the shared test for Q68, Q69, Q211, Q206, Q207, and Q74.

**Durability order.** Folded into Q74. A shutdown write on its own does not prove that a periodic write happened, or that an earlier dirty snapshot cannot resurrect a deleted group. The order in citation 9 is the proof. Baked and live terrain writes stay on Q217's own timing. The ordinary default-parcel writer and the archive's non-default writer share one store contract and use different triggers (D39).

**Category before and after restart.** [docs/found/Q208.md](../found/Q208.md) records that a loaded archive can carry a nonzero object Category that the current model omits. C# reads it from XML (`SceneObjectSerializer.cs` 1043–1046). It need not survive a SQLite restart. A comparison taken only after restart would hide the immediate difference. Q384 includes a nonzero case and compares each server at the same stage, or it records the accepted difference. A prim delete capture can contain two kills for the root (`SceneObjectGroup.cs` 2983–2986). An avatar-departure assertion of exactly one kill is the wrong assertion for prim deletion.

### Forecast

The independent review counted 35 outstanding named leaves on the morning snapshot, including Q81 and excluding optional Q76b, at roughly 40 to 49 focused coder-days, plus a further 3 to 6 days if no row absorbed the cross-backend reads, the integrated archive load, and the combined gate rehearsal. This revision assigns those three to Q383, Q384, and Q385, and it adds Q382 and Q386 and Q387 to the leaf list. The review's window stands. P2.12 is ready for the maintainer between 11 and 17 December 2026, with G3 after it. That reading uses four lanes on calendar days starting 27 November 2026, the object spine and the parcel and restore tail at about ten days in sequence, two to four review rounds, timely reviews, and no expansion to riders, attachments, or script execution. The same-day merges already on main are independently prepared work. They are not a rate for the dependent writers. Merged animation, parcel, and child work is not charged again.
