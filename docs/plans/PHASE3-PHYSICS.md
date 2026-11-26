# Phase 3 plan: physics

Planning note for the manager. No queue rows exist yet ([PLAN.md](../PLAN.md), 25 November 2026). This file is the map those rows come from. It describes Tranquillity at Pin8 `c401bceecd1684832ae8ba86a31b5ee01a4ded7b` and Ferroan at the commit this branch was cut from.

**Gate** ([PLAN.md](../PLAN.md)): vehicle, walking and collision tests give the same results as the C# Jolt module.

**Scope there:** Jolt behind `ferroan-physics`, prim, sculpt and mesh shapes, the avatar controller, vehicles, buoyancy and collision events. The arrival landing ray cast is part of that scope (the note under Phase 3 in PLAN.md).

**Standing decisions**

- [D6](../DECISIONS.md). The region frame, Jolt and SLua each run on their own threads. Jolt is not an async task.
- [D8](../DECISIONS.md) and [AGENTS.md](../../AGENTS.md). The only native library in this phase is Jolt, through `oxijolt-sys`, from the one crate `ferroan-physics`. Every `unsafe` block there needs a `SAFETY` comment covering ownership, lifetime, threading and bounds. The workspace keeps `unsafe_code = "forbid"` everywhere else.
- [D9](../DECISIONS.md). Jolt only. BulletSim, ubODE, POS and basic physics are not ported. [D38](../DECISIONS.md) deletes the BasicPhysics walking stand-in when Jolt arrives. The stand-in is not a selectable engine.
- [PLAN.md](../PLAN.md) building blocks: `oxijolt-sys` 1.5, Jolt 5.6 through joltc. Convex decomposition is listed there as parry3d's V-HACD. The Pin8 Jolt module does not use that algorithm. See the open questions.

The phase estimate stays 6 to 11 person-months. The rows below are a first cut at about a day each, in dependency order. A row that overruns a day is split before the next one starts, as Phase 2 did. `VehicleController.cs` (4,133 lines) and `JoltPhysicsBackend.cs` (5,566 lines) are the files most likely to split again.

Line numbers below are the file as it is at Pin8. Paths are under `Source/` in the Tranquillity checkout.

## 1. What the C# Jolt module does

### Binding and Jolt version

The region module does not call Jolt C++ itself. It calls `JoltPhysicsSharp` 2.19.1, which P/Invokes `joltc`. The native package is `JoltPhysics.Native` 1.0.4, pinned in `OpenSim.Region.PhysicsModules.Jolt.Backend.csproj` (lines 31–38). The package records commit `ba2f3068c391d33788d33ebe53dc0ad4f7c15be4` of JoltPhysicsSharp (10 October 2025), the commit that exposed `HeightFieldShapeSettings`. The linux-x64 library is `libjoltc.so`, 2,046,568 bytes, dated 10 October 2025. It carries no version string.

The last Jolt release the binding named before that pin is 5.4.0: JoltPhysicsSharp 2.18.6, 29 September 2025, "native joltc (with v5.4.0 support)". Jolt 5.5.0 is 28 December 2025. Jolt 5.6.0 is 11 July 2026. The Pin8 native is the 5.4 generation. It is not Jolt 5.6.

`JoltNative.cs` (1–338) loads one `joltc` per process, checks its SHA-256, and refuses an unrecorded file unless `[Jolt] AllowUnrecordedNative` is set. Every recorded build has `SafeForMultiplePools` false: the stock library gives every physics system one process-wide temp allocator, so two `PhysicsSystem` updates at once stop the process. The backend therefore runs one job pool whatever `[Jolt] JobPools` asks (`JoltPhysicsBackend.cs` 22–25, `ResolveJobPools` 389–391).

The module runs single precision (`Foundation.Init(false)`, noted in `JoltPhysicsBackend.cs` line 12 and in `JoltNative.targets`). The double-precision `joltc_double` binaries are dropped from the build output.

`JoltPhysicsSharp` is MIT. `JoltPhysics.Native` / joltc is MIT. Jolt itself is MIT. Ferroan does not port the C# binding. It binds Jolt 5.6 through `oxijolt-sys` (MIT OR Apache-2.0), already named in [NOTICE.md](../../NOTICE.md).

### Threads, and how the region frame calls the scene

Jolt does not have a thread of its own in C#. `Scene.Heartbeat` (`Scene.cs` 1684–1716) starts one thread per region, `Heartbeat-(name)`. `Scene.Update` (1731–1823) calls `SceneGraph.UpdatePhysics` (`SceneGraph.cs` 222–224), which calls `PhysicsScene.Simulate(FrameTime)`, on that heartbeat thread. Shipped `FrameTime` is 0.0909 s (`RegionServer.ini` via `Scene.cs` 1145). The constructor's 0.0908 applies only before the ini read (`Scene.cs` 863). Ferroan's frame already uses 0.0909 (`crates/ferroan-scene/src/frame.rs`).

Inside one heartbeat, before physics: terrain taint every fourth frame, `UpdateTerrain` every `m_update_terrain` frames. After physics: `CheckAtTargets`, moving-start/end, object-group updates, then `UpdatePresences` (the terse updates viewers see). Physics results of a heartbeat are visible to presence updates in the same heartbeat.

`JoltScene.Simulate` (`JoltScene.cs` 2786–2834) is the single-step path. The shipped path is the other one. `[Jolt] PhysicsStepRate` defaults to 45 in `JoltConfig.cs` (194) even when the key is left commented in `RegionServerDefaults.ini` (1417–1427). `EffectivePhysicsStepRate` (361–379) keeps 45 whenever it is at least the heartbeat rate. A 0.0909 s heartbeat is about 11 Hz, so the default is 45 Hz. `PhysicsStepRate = 0` is the single step per heartbeat.

`SimulateSubsteps` (3096–3156) and `SubstepAccumulator` (1–76):

- Once per heartbeat, first: rebuild dirty linkset compounds, answer queued camera rays, activate bodies that were created inert.
- The accumulator adds the heartbeat time and takes as many steps of exactly 1/45 s as it holds, at most 16. The remainder carries. A long run at 45 Hz is 45 steps per second. One heartbeat takes 4 or 5 of those steps. The module's own comment says 11 Hz heartbeats at 45 Hz alternate 4 and 5 (`RegionServerDefaults.ini` 1417–1419). The oracle has to use the same `FrameTime` float and the same accumulator, starting from a zero carry, so the split matches.
- Every 1/45 s step, on the heartbeat thread: keyframed motion for the time the heartbeat covers (once), then vehicle controllers, script forces and move/hover targets, then `IPhysicsBackend.Step`.
- `Step` (`JoltPhysicsBackend.cs` 5022–5106) takes the job-pool gate, then `_simLock` for the whole step. Under that lock it steps every `CharacterVirtual` first (`StepCharacter`, 4075), then `PhysicsSystem.Update` for rigid bodies. Characters see the world as it was at the start of that step.
- Once per heartbeat, last: drain body and avatar state from the last step only, and dispatch the contacts of every step in the heartbeat as one set (`mergeSubsteps: true`). Viewers and collision events therefore still update once per heartbeat.

With the 45 Hz path on, solver sub-steps come from `PhysicsStepCollisionSteps` (default 2), raised so each solver slice is at most 1/60 s (`CollisionStepsAt`, `JoltConfig.cs` 409–412). At 45 Hz that is 2, so the solver runs at 90 Hz. `CollisionSteps = 6` (the ini default) applies only when `PhysicsStepRate` is 0: six solver slices inside the one 0.0909 s update, and the character still steps once.

Jolt's own worker threads exist (`JobSystemThreadPool`, `JoltPhysicsBackend.cs` 87–120). They run inside `Update`. They are not the region heartbeat, and they are not safe for two physics systems at once on this native. `DeterministicMode` defaults to false (`JoltConfig.cs` 43).

Ray casts from scripts take the same `_simLock`, so they do not share the temp allocator with `Update` (`JoltScene.cs` 2241–2246).

### File by file

**`OpenSim.Region.PhysicsModules.Jolt`** (the region module, MPL-2.0, Legion Builds 2026). Found when `[Startup] physics` is exactly `Jolt` (`JoltModule.cs` 30–56, `PluginRegistration.cs`).

| File | Lines | What it does |
| --- | --- | --- |
| `JoltModule.cs` | 76 | Loads nothing unless physics is `Jolt`. Then one `JoltScene` per region. |
| `PluginRegistration.cs` | 45 | Plugin registry entry. |
| `JoltConfig.cs` | 521 | Every `[Jolt]` knob. Defaults are the shipped behaviour. Invalid values warn and fall back. Gravity −9.80665. See the step-rate section above. Avatar speed factors turn `ScenePresence`'s request into the documented walk 3.20, run 5.13, fly 16.00 and fly-down 22.87 m/s (179–182). |
| `JoltScene.cs` | 3,735 | The `PhysicsScene`. Initialise, simulate, terrain, rays, collisions, shape cooking, console. |
| `JoltPrim.cs` | 2,816 | One prim actor: body, mass, buoyancy, phantom, volume detect, link weld, vehicle parameter entry points. |
| `JoltPrim.Kinematic.cs` | 448 | Keyframed (grab / llSetKeyframedMotion) motion over the step. |
| `JoltPrim.Targets.cs` | 410 | Move-to-target, hover, ground repel. |
| `JoltPrim.AngularTarget.cs` | 257 | Rotation look-at. Does not change gravity or buoyancy. |
| `JoltCharacter.cs` | 598 | Avatar actor over a `CharacterVirtual`. |
| `JoltSitAvatar.cs` | 349 | Sit and stand. `[Jolt] PhysicsSit` selects the physics sit path. |
| `JoltVehicleBody.cs` | 167 | The prim body as the vehicle controller's `IVehicleBody`. |
| `VehicleStepProvider.cs` | 180 | Applies the controller's force inside the backend step. |
| `BasicForcesStepProvider.cs` | 559 | Hover, move-to-target, ground repel and buoyancy-scaled gravity during the step. |
| `HalcyonBasicForces.cs` | 285 | InWorldz formulas, including the buoyancy hack: a value from 0.977 up to 1 becomes 1 (97–104). |
| `ScriptForceStepProvider.cs` | 151 | `llSetForce` / `llSetTorque` on a prim, reattached when the body is recreated. |
| `PrimHullCells.cs` | 697 | A physical concave prim as a compound of convex cells from its profile rings. This is the convex decomposition the Jolt module actually runs. |
| `SubstepAccumulator.cs` | 77 | 45 Hz step count per heartbeat. |
| `CollisionFrameTracker.cs` | 215 | Per-frame touching set, collision score, sleeping contacts held. |
| `JoltMetrics.cs` | 334 | Step timings for `jolt metrics`. |
| `CapacityReport.cs` | 141 | `jolt capacity` text, including job-pool gate wait. |
| `JoltDiagnosticsLoader.cs` | 115 | Loads the diagnostics assembly only when `[Jolt] TestCommands` is true. The region server does not ship that assembly. |
| `NonFiniteGuard.cs`, `RepeatLimiter.cs`, `RayCastTimeExceededException.cs` | 75, 52, 30 | Reject non-finite script values, rate-limit warnings, `llCastRay` time-budget failure. |

**`OpenSim.Region.PhysicsModules.Jolt.Backend`** (MPL-2.0). No OpenSim reference. `IPhysicsBackend.cs` (1,211) is the engine-shaped API: body, shape, character, ray, step result, layers, contact phase. `JoltPhysicsBackend.cs` implements it. `JoltPhysicsBackend.RayCasts.cs` (387) is the budgeted ray cast. `JoltPhysicsBackend.StepProviders.cs` (225) runs vehicle and script forces inside `Update`. `StepForceProvider.cs` (108) is that callback. `JoltNative.cs` is the loader above.

Shape constructors: box 1688, sphere 1696, capsule 1701, cylinder 1707, convex hull 1719, triangle mesh 1734, compound 1768, hull compound 1812, heightfield 1842, scaled 1935. Bodies 2034. Characters 3000. `Step` 5022. Terrain body swap `SetTerrain` 4515. Water `SetWaterHeight` 4577. Character buoyancy 4401.

**`OpenSim.Region.PhysicsModules.Jolt.Vehicles`** (MPL-2.0). `VehicleController.cs` (4,133) is the controller. `ProcessTypeChange` 462, float/vector/rotation parameters 513–740, flags 799, `Step` 1018, `StepOnce` 1132, `StepCore` 1175, gravity 1808. `HalcyonVehicleLaws.cs` (647) is the InWorldz force laws. `VehicleProperties.cs` (247) holds type, flags and parameters. `VehicleParamCodes.cs` (128) is the LSL integer codes. `VehicleSettings.cs` (222) is presets and the InWorldz/SecondLife behaviour switch. `VehicleLimits.cs` (147), `VehicleMotorSolver.cs` (138), `VehicleSpring.cs` (83), `VehicleDynamicsState.cs` (173), `IVehicleBody.cs` (110).

**`OpenSim.Region.PhysicsModules.Jolt.Diagnostics`** (MPL-2.0, optional). `JoltTestCommands.cs` (2,447) rezzes, drops and drives. `JoltDiagnostics.Drops.cs` (109) tracks `droptest`. `JoltDiagnostics.Scene.cs` (42) is the read surface. `SledSlideScenario.cs` (138). Not loaded in production.

**`OpenSim.Region.PhysicsModules.Meshing`** (BSD). `Meshmerizer.cs` (1,154) is the `IMesher`: prim, sculpt and mesh assets. `PrimMesher.cs` (2,449) builds a prim from profile and path. `SculptMesh.cs` (622) and `SculptMap.cs` (178) turn a sculpt image into a mesh. `SculptMap` decodes pixels with SkiaSharp (a native library). `Mesh.cs` (414) holds triangles and, for a mesh asset, the convex-hull list already stored in the asset. `ZeroMesher.cs` (139) is the empty mesher. Jolt treats a missing mesher as a bounding box (`JoltScene.cs` 1564–1568).

**`OpenSim.Region.PhysicsModules.ConvexDecompositionDotNet`** (MIT, Intel 2010, about 6,400 lines). A C# port of John W. Ratcliff's convex decomposition. Referenced by ubODE, ubODEMeshing and BulletS, and linked into the region server for those engines. The Jolt projects do not reference it. [D9](../DECISIONS.md) leaves it out.

**`OpenSim.Region.PhysicsModules.SharedBase`** (BSD). The scene-facing contract, not an engine.

| File | Lines | What the scene calls |
| --- | --- | --- |
| `PhysicsScene.cs` | 386 | `AddAvatar`, `AddPrimShape`, `RemoveAvatar`, `RemovePrim`, `Simulate`, `SetTerrain`, `SetWaterLevel`, `DeleteTerrain`, `GetTopColliders`, ray casts. `RayFilterFlags` at 45. `Initialise` (118–123) sets the asset callback, then terrain, then water. |
| `PhysicsActor.cs` | 833 | Pose, velocity, force, buoyancy, vehicle parameter calls, collision subscription, `SendCollisionUpdate`. `CollisionEventUpdate` at 116. |
| `VehicleConstants.cs` | 265 | LSL `Vehicle` codes and `VehicleFlag` bits. The saved `VehicleData` record. |
| `IMesher.cs`, `IMesherShapeData.cs` | 95, 41 | What `Meshmerizer` implements. |
| `PrimProfileRings.cs` | 85 | Profile rings `PrimHullCells` consumes. |
| `IPhysicsParameters.cs` | 67 | osSetPhysicsParam style entries. `Scene.cs` 109 reads them when the engine implements the interface. |
| `PhysicsJoint.cs`, `PhysicsSensor.cs`, `CollisionLocker.cs`, `NullPhysicsScene.cs` | 51, 76, 69, 101 | Joints are not used by the Jolt module. Volume detect is a body layer, not `PhysicsSensor`. |

### Actors

**Avatar.** `JoltScene.AddAvatar` (1044–1150) builds a `JoltCharacter` (`JoltCharacter.cs`). The body is a Jolt `CharacterVirtual`, not a solved rigid body (`JoltPhysicsBackend.cs` 3002–3036). A capsule: radius from the footprint, at least 0.2 m, cylinder half-height what remains of the box (`JoltCharacter.cs` 115–120). The default appearance box is 0.45 × 0.6 × 2.1 (the 1.9 m height plus the 0.2 m box adjust). Mass is 1.954922 scaled by capsule volume against that default (123–138). A kinematic query marker with the avatar's local id lets a ray find the capsule. `ScenePresence` writes `TargetVelocity`, `SetAlwaysRun`, `Flying` and a latched jump. The drain (`ApplyCharacterState`, 183) writes position, velocity and ground state back and requests a terse update. `IsColliding` is support. Ground is terrain when the supporting body is the terrain body or there is no body.

**Prims.** `AddPrimShape` (`JoltScene.cs` 1150–1176) cooks a shape and makes a `JoltPrim`. Non-physical prims are static. Physical prims are dynamic and are created inert, then activated at the start of the next simulate, after link weld and vehicle setup, so a reloaded vehicle does not free-fall (`JoltScene.cs` 2748–2810). Phantom is layer `Phantom`. Volume detect (`llVolumeDetect`) is layer `Sensor` and wins over phantom (`JoltPrim.cs` 50–54, 242–243). A selected or held physical prim is kept still (1857–1891). A physical prim whose centre is below the terrain is lifted onto it before the body exists (`TryUnburyPhysicalLoad`, `JoltScene.cs` 3648–3664).

**Linksets.** Each part is its own actor. `child.link(root)` (`JoltPrim.cs` 1991–2028) only marks the root dirty. `Simulate` welds before the step (`DrainDirtyLinksets`, `JoltScene.cs` 2768–2784): children become shapes in the root's `StaticCompoundShape`, and their own bodies go away (`RebuildCompoundNow`, `JoltPrim.cs` 2070). Collision identity of a child is the compound child's user data, so `llDetectedLinkNumber` is that link. Mass is the sum of each part's volume times that part's density (1345). A child script's buoyancy is forwarded to the root (`JoltPrim.cs` 1784–1794).

**Attachments.** A worn attachment is not a second physics body in this module. `llSetForce`, `llApplyImpulse`, `llPushObject`, `llSetBuoyancy` and `llSetHoverHeight` on an attachment are applied to the wearer's `JoltCharacter` (`JoltCharacter.cs` 461–518, and the force path in `StepCharacter` 4099–4136). Dropping an attachment, or handing back a saved vehicle, goes through `JoltPrim.SetVehicle` (2363–2370) and the normal prim add path. Grabbing and keyframed motion of an attached prim are the wearer's problem on the scene side. Phase 3 ports the wearer-side forces. The script calls that produce them arrive with Phase 4.

### Shapes

`JoltScene` cooking starts at 1186.

An uncut box, sphere or cylinder (no profile/path cut, hollow, twist, taper, shear, and path scale 100) is an analytic shape (`PrimHasNoCuts` 2224–2228, fast path 1229–1258): `CreateBoxShape`, `CreateSphereShape`, `CreateCylinderShape`. Anything else, and every sculpt or mesh, goes through the mesher.

`CookMeshShape` (1282) and `CookMeshShapeCore` (1546):

- A non-physical prim becomes a triangle `MeshShape`. A physical prim becomes one convex hull of those points, because a triangle mesh reports volume 0 and would have mass 0 (1710–1711).
- A physical prim that is not a mesh asset and not a sculpt, when a hull list is wanted, is cooked as convex cells (`PrimHullCells.Build`, `JoltScene.cs` 1869–1910). The cells are the prim's real volume for mass.
- A mesh asset uses the hull list the asset already carries (`Mesh.ConvexHulls`), shared by asset id and size (`SharedShapeKey` 1314). A non-physical mesh whose physics shape type is convex stays a triangle mesh unless `[Jolt] HonourConvexMesh` is true (`JoltConfig.cs` 106–108). A non-physical mesh under 0.05 m on every side is a box (1556–1562). A mesh whose asset has not arrived yet stays a box and is recooked when the asset lands (`RequestMeshAssetIfNeeded`, 1181).
- A sculpt whose texture has not been fetched meshes to null and falls back to a box (1610–1612). The mesher builds an inverted sculpt inside out, as the viewer draws it, and the cook flips it for collision (1649).
- Identical cooks are cached. The mesher returns one shared `Mesh` instance per key. The scene copies vertices out and does not call `releaseSourceMeshData` on that instance (1571–1579).

`PrimMesher` is the geometry. `SculptMesh` / `SculptMap` are the sculpt image. There is no call into `ConvexDecompositionDotNet`.

### Avatar controller

`StepCharacter` (`JoltPhysicsBackend.cs` 4071–4249), once per 1/45 s physics step on the default path:

- **Walking.** Horizontal velocity is the requested velocity, scaled by the walk or run factor in `JoltCharacter` before it is stored. On walkable ground the avatar adopts the ground's horizontal velocity (a moving platform) and does not keep a slide from the previous step.
- **Flying.** `Flying` gives full 3D control. Gravity is off. Character mass for pushing objects drops to 0.001 so a flying avatar does not press a light object with a standing avatar's weight (4229–4235). Vertical speed uses the fly and fly-down factors.
- **Falling and landing.** On walkable ground, gravity is not accumulated, so the avatar stands on a slope inside `MaxSlopeAngle` (`OnGround`) and does not creep. In the air, or on steep ground (`OnSteepGround`, `IsSliding`), gravity is −9.80665 scaled by `(1 − buoyancy)`, applied as half a step before the move and half after if still in the air (4172–4246). Landing is the next step finding walkable ground and taking that ground's vertical velocity.
- **Jump.** A latch in `JoltCharacter` (51–55), because `ScenePresence` clears vertical intent before the step. Jump speed defaults to 4 m/s, and only from walkable ground (4158–4160).
- **Stairs.** `ExtendedUpdateSettings.WalkStairsStepUp` is the avatar's `StepHeight` along +Z (4223–4227). Jolt's character defaults are Y-up. The module remaps them to Z-up. Stick-to-floor pulls down by the same step height so a ramp does not leave the feet floating, except while hovering or while buoyancy holds the avatar up (`gz >= 0`). The avatar does not step up onto a loose dynamic object in its path. It pushes that object (`LooseObjectInStep` 3228, `MoveOutOfCapsule` 3256).
- **Speed the viewer asked for** is still `ScenePresence.AddNewMovement` (about 4.096 m/s walk, four times that flying). The factors in `JoltConfig` are what make the controller match the documented metres per second. Phase 2 already stores that intent (`P2.2a`).

### Vehicles

A vehicle is a physical prim whose actor owns a `VehicleController`. Parameters and flags enter through `PhysicsActor` (`JoltPrim.VehicleFloatParam` and the three siblings, 2335–2361) and through `SetVehicle` for a saved record (2382). The controller runs at the start of each 1/45 s step (`StepVehicles` inside `SimulateSubsteps`), and a `VehicleStepProvider` applies the resulting force during `PhysicsSystem.Update`.

Types (`VehicleProperties.cs` 139–151): none, sled, car, boat, airplane, balloon, plus InWorldz motorcycle (LSL 10002) and sailboat (LSL 10001).

Flags (`ExtendedVehicleFlags`, 158–184) use the OpenSim `VehicleFlag` bit values through `LockRotation` (32768), then Halcyon extensions: `LimitMotorDown`, `TorqueWorldZ`, `MousePointSteer`, `MousePointBank`, `ReactToWind`, `ReactToCurrents`. Standard names: `NoDeflectionUp`, `LimitRollOnly`, `HoverWaterOnly`, `HoverTerrainOnly`, `HoverGlobalHeight`, `HoverUpOnly`, `LimitMotorUp`, `MouselookSteer`, `MouselookBank`, `CameraDecoupled`, `NoX`, `NoY`, `NoZ`, `LockHoverHeight`, `NoDeflection`, `LockRotation`.

Float parameters (83–104): the usual LSL deflection, banking, buoyancy, hover and vertical-attraction values, plus InWorldz `InvertedBankingModifier`, `MouselookAltitude`, `MouselookAzimuth`, `BankingAzimuth`, `DisableMotorsAbove`, `DisableMotorsAfter`. Vector parameters (112–125) keep a timescale per axis, which is the Halcyon model, plus wind efficiencies. One rotation parameter: the reference frame.

`[Jolt] VehiclePresets` defaults to `Documented` (the wiki presets). `[Jolt] VehicleBehaviour` defaults to `InWorldz` (`JoltConfig.cs` 65–66, `RegionServerDefaults.ini` 1439–1448). The ini comment lists where those differ: a tenth of pitch righting kept on cars, boats and sleds under `LimitRollOnly`, hover-up behaviour while buoyant, and friction that would push a vehicle upward being dropped. `VehicleGroundGravityFactor` defaults to 1 (full gravity on a car or sled that is touching something). Sled contact friction defaults to 0.

Boat and balloon buoyancy, hover height and water level (`SetWaterLevel`, `JoltScene.cs` 3691–3694) are the first vehicle behaviours a viewer notices. Linear motor, angular motor, banking and deflection are the rest of `StepCore`.

### Buoyancy

`llSetBuoyancy` on a prim stores a factor. The body's gravity factor is `(1 − buoyancy) × gravity multiplier`, clamped (`JoltPrim.cs` 1607–1818). InWorldz behaviour maps 0.977 up to 1 onto 1. A vehicle ignores the prim's `llSetBuoyancy` and uses `VEHICLE_BUOYANCY`. The value is kept for when the vehicle type is cleared. On an avatar, the same `(1 − buoyancy)` scales gravity, and it does nothing while flying or hovering (`StepCharacter` 4089–4091, `JoltCharacter.cs` 499–518). Water height is a separate input for vehicle hover (`HoverWaterOnly`), stored by `SetWaterLevel`. Buoyancy itself is a gravity scale, not a fluid force.

### Collision events

`DispatchContacts` (`JoltScene.cs` 3237–3416) runs on the heartbeat thread after the drain. Each subscribed prim gets one `CollisionEventUpdate` listing local ids it touches this heartbeat. Terrain is local id 0. `SceneObjectPart.PhysicsCollision` diffs that set against the previous heartbeat and fires `collision_start`, `collision` and `collision_end`. `llDetected*` reads the list. A pair that begins and ends inside one heartbeat still counts as a start, then an end on the next heartbeat. Contacts from the 4 or 5 inner steps are merged. A pair is counted once.

Delivery is to the struck part (compound child), and the other side is named by the body (a physical linkset by its root). Volume-detect contacts go only to the detector. Avatar contacts name land as 0 and an object by its root. Two avatars each hear the other, because Jolt reports the contact on only one character update (3355–3362). A contact whose normal points down is the feet, and becomes the collision plane under the avatar. Relative speed along the normal feeds collision sounds (above 0.2 m/s) and impact damage (below −5 m/s).

A pair that is missing this heartbeat only because both bodies slept is held (`StillTouchingAsleep`, 3541–3549), so a resting object does not get `collision_end` when it falls asleep. `GetTopColliders` (3705–3718) returns up to 25 prims by contact count.

Phase 4 is what runs the script events. Phase 3 has to produce the same `CollisionEventUpdate` the scene would hand to `SendCollisionUpdate`, and the collision plane on the avatar terse update.

### Ray casts

`SupportsRaycastWorldFiltered` is true (`JoltScene.cs` 2247). That is the flag `ScenePresence.MakeRootAgent` checks.

Two world entries (`2274–2284`):

- The 4-argument cast is a script `llCastRay` (Phlox). Filter is terrain, static, dynamic and avatar. Phantom and sensor are excluded. Charged to the script budget. A budget miss throws `RayCastTimeExceededException`.
- The 5-argument cast takes `RayFilterFlags`. Callers are rez placement (`Scene.GetNewRezLocation`), the arrival landing ray (`ScenePresence.MakeRootAgent`, `ScenePresence.cs` 1543–1602), and YEngine `llCastRay`. Charged to the simulator budget. A miss returns no hits.

`RaycastActor` (2293) is the sit query: one part, including a welded child as its own shape. The viewer camera ray does not use `RaycastWorld`. The module does not override `SupportsRayCast`, so the base class answers the camera. Camera rays that the module does queue are drained at the start of simulate (`DrainCameraRays`, 2708).

Budgets are `[Jolt] RayCastBudgetMs`, `RayCastSimulatorBudgetMs`, `RayCastMaxTestedHits` and `RayCastMaxHits` (cap 256). They reset every heartbeat (`BeginRayCastBudget`, `Simulate` 2794).

The arrival ray casts down from above the arrival point with `RayFilterFlags.PrimsNonPhantomAgents` when the arrival is not carried, the landing point was not moved, and the engine supports filtered rays. The newcomer stands on the first prim or avatar capsule hit. A second avatar at the same point stands on the first, about 2 m up. BasicPhysics returns false from `SupportsRaycastWorldFiltered` (`PhysicsScene.cs` 347–350), which is why the Phase 2 stand-in has no ray and why a `--physics basicphysics` capture lands at ground height.

### Terrain heightfield

`SetTerrain` (`JoltScene.cs` 3552–3613) takes the region's `float[]` in OpenSim order, `heightMap[y * sizeX + x]`. It builds an `(N+1)` square field so the heightfield spans the far edge. A 256 m region becomes 257 samples at 1 m. Non-square regions pad to `max(sizeX, sizeY) + 1` by repeating the last sample. `CreateHeightFieldShape` plus `SetTerrain` swaps the terrain body. The cooked samples stay in `_terrainField` for `TerrainHeightAt` (vehicle hover and the unbury test), so the reported height is the collision surface. A terrain edit lifts avatars that would be buried (`ReGroundAvatarsOnTerrainChange`, 3672). It does not pull avatars down. Water is `SetWaterLevel` only.

The heightmap Ferroan already loads (`P2.0e`, `P2.0e2`, `P2.0e3`) is this array.

### What diagnostics exposes

Always registered, read-only (`JoltScene.cs` 626–647): `jolt capacity`, `metrics`, `terraintest`, `probe <x> <y>`, `heights <x> <y>`, `avatarstatus`, `charframe [secs]`, `sitstatus`, `sensortest`, `raytest`, `reloadcheck`, `vehiclestatus`. None of them change the region. `probe` and `terraintest` ray cast the heightfield. `avatarstatus` prints support, ground and velocity. `vehiclestatus` prints type, buoyancy and whether the controller is active (`JoltPrim.cs` 193–198). `charframe` logs per-step avatar Z for a few seconds.

Loaded only with `[Jolt] TestCommands` (`JoltTestCommands.cs` 40–66): `linktest`, `unlinktest`, `collidetest`, `collidelinktest`, boat/car/sled/plane/balloon tests, `terrainslope`, `terrainhill`, `hilltest`, `rezprims`, `rayprims`, `rezmesh`, `raymesh`, `droptest`, `dropmesh`, `dropstatus`, sit tests, `clearprims`. `jolt parity [terrain|ramp|drop|core|boat]` (108–116) writes `parity-<engine>.txt` with rest position, settle frames and mass, so two engines can be compared by running the same command. That command is the C# side's own comparison harness. It lives in the optional assembly.

## 2. What Ferroan has today, and what Phase 3 replaces

`ferroan-physics` does not exist. Nothing in the workspace depends on `oxijolt-sys`. The workspace forbids unsafe code. [NOTICE.md](../../NOTICE.md) already names Jolt and `oxijolt-sys` as planned. Their licence texts are added when the dependency lands.

What touches motion today:

- **Terrain heights.** `ferroan-scene` loads the heightmap as C# `Scene.LoadWorldMap` does: the `terrain` row, else `bakedterrain`, else `[Terrain] InitialTerrain`, on SQLite, MySQL and PostgreSQL (`P2.0e`, `P2.0e2`, `P2.0e3`). The frame samples it for login and teleport landing and for the walking stand-in. Phase 3 keeps that array and also cooks it into a Jolt heightfield. The interpolated height used at login stays the scene's until the arrival ray replaces the "land on the ground" part of that rule.
- **The frame thread.** One thread, `FrameTime` 0.0909, no drift (`P2.1a`, [D6](../DECISIONS.md), [D39](../DECISIONS.md)). It owns presences and stored prims. Circuits post `AgentUpdate` and do not block on the thread. After each tick it applies terse thresholds and queues viewer updates. Phase 3 does not move that thread. It adds a second thread for Jolt. The frame asks that thread for one heartbeat and waits for the poses before it samples them for terse updates, so a viewer still sees the post-physics position in the same tick. Waiting on the physics thread is a thread join, not an async task.
- **Movement intent.** `P2.2a` stores `AgentUpdate` as target velocity, fly, always-run and the control flags, with C#'s significance filters. Phase 3 feeds that intent to the character controller instead of to the stand-in.
- **The walking stand-in.** `crates/ferroan-scene/src/walk.rs`, from BasicPhysics `Simulate` (`P2.2b1`, `P2.2b2`). Each tick, X and Y move by velocity times `FrameTime`. Z is the integer terrain cell plus the 2.1 m box while walking, a fall to that height, or a held Z while flying, with a snap when flight would go through the ground. Prims are ignored. There is no collision plane (`frame.rs`). `fixtures/walk/walk.cap` is a `--physics basicphysics` capture, and the stand-in matches those positions bit for bit. The live walk scenario uses that flag on the C# region. Ferroan has no physics switch ([LIVE.md](../LIVE.md)).

Phase 3 deletes the stand-in ([D38](../DECISIONS.md)) once the character controller matches a Jolt capture of the same walk. Until that row, the stand-in still moves avatars, and Jolt can already be stepping a falling box on the other thread. The two must not both write an avatar's position.

Stored prims (`P2.4a`, `P2.5a`) are scene objects with a physics flag and a shape. They do not collide and they do not move. Phase 3 is what makes a physical one fall and a static one solid. Terse updates for prims already have a queue (`P2.5a`). The physics drain is a new producer on that queue.

Water height and region settings (`P2.4c`) are the water level `SetWaterLevel` needs. Parcel and permission work does not block physics. Animations (`P2.3b1`, still TODO) read "on ground" from the stand-in's colliding flag today. After the stand-in goes, that flag is the character's `IsSupported`.

The arrival landing rule in the scene is the only copy, and it has no ray ([PLAN.md](../PLAN.md)). Phase 3 adds the ray beside that rule and lands on the first hit. Comparisons of arrival against a C# region stay on `--physics basicphysics` until that fixture exists. The harness default C# engine is ubODE, which does cast the ray, so a default C# arrival is the wrong oracle for Ferroan until both sides run Jolt.

## 3. Proving equivalence with C#

### What can be compared tightly

Same inputs, same timestep, same algorithm, on one machine:

- The heightfield samples: the `(N+1)` expansion of a known `float[]`, then a downward ray at integer metres. This is geometry. It should match to a small multiple of a millimetre once both sides build the same field, including the far edge.
- The analytic box, sphere and cylinder, at rest, not touching anything else. Pose in, pose out.
- The 45 Hz accumulator: how many 1/45 s steps each 0.0909 s heartbeat takes, from carry zero. That count is pure arithmetic (`SubstepAccumulator.Advance`) and can match exactly, in integers.
- Collision-event identity: which local ids, which phase (start, persist, end), which heartbeat index. The set of names in a scripted collision is exact even when the contact point is not.
- Vehicle flags and parameter stores after a scripted sequence of `llSetVehicleType`, `llSetVehicleFloatParam` and flag calls. The stored numbers are exact. The resulting motion is not.

Jolt 5.6 changes the friction model, and the project notes that the simulation moves a little because of it. Pin8's native is the 5.4 generation. A falling box that never touches a surface, in a vacuum with the same gravity and the same 1/45 s steps, is the closest the two versions come. The moment the box meets the heightfield, the contact solver is allowed to differ.

`DeterministicMode` is off in the C# defaults, and the job system is free to schedule contacts differently across runs of the same binary. Treat one C# capture as the oracle, recorded once, and compare Ferroan with that capture. Re-running C# to refresh the capture is a conscious step, not something CI does on every build.

### What needs a tolerance

- Centre position and linear velocity of a body, per heartbeat, after the first contact.
- Rest height and the heartbeat index at which a dropped box sleeps.
- Avatar feet position while walking, flying, falling, landing and climbing a step. The capsule and the stair solver will not hit the same float as 5.4.
- Vehicle telemetry: position, linear and angular velocity, motor force, buoyancy, hover error. Record every heartbeat, compare with a tolerance that the first capturing row measures and writes down. Do not invent the tolerance in advance.
- Contact position and normal. The event (who touched whom) is exact. The point on the surface is a tolerance.

A tolerance is a number of metres or metres per second on the captured channel, plus the heartbeat index. It is stored next to the fixture. Tightening it is a later row, after someone has looked at the residual.

### What to capture

Prefer an in-process oracle, on the pattern of `tools/regionstore-oracle`: a small C# program that references the pinned Jolt projects, builds one `JoltScene` with the shipped `[Jolt]` defaults (so `PhysicsStepRate` stays 45), steps a fixed number of heartbeats of 0.0909 s, and writes JSON. No region server, no viewer, no sockets. `cargo xtask jolt generate` and `verify` run it. Ordinary CI does not run it. The Linux live workflow can, the way it runs the other oracles. Rust tests replay the committed JSON on every platform and skip, with a printed reason, when the native library for that platform cannot be built. [TESTING.md](../TESTING.md) already says a documentation-only pull request does not start a server, and that native crates get `SAFETY` comments and fuzzing of the boundary.

Capture, per case:

- **Positions per tick.** Heartbeat index, physics-step index inside it, position, linear and angular velocity, whether the body is active. The falling box, a box at rest on flat terrain, a box on the pinhead heightmap, a box on a slope.
- **Collision events.** Heartbeat index, the two local ids (0 for terrain), phase, relative speed. The C# `collidetest` is the scene-level version of this. The oracle can read the `CollisionEventUpdate` the actor was given, which does not need a script.
- **Vehicle telemetry.** Heartbeat index, type, flags, buoyancy, linear and angular velocity, position, the motor direction the controller is applying. One short run per type (the `boattest` / `cartest` / `sledtest` / `planetest` / `balloontest` cases), driven by parameter calls, not by a viewer key.

`jolt parity` is the same idea already living in the diagnostics assembly. The oracle may call that code, or it may step the scene itself. Either way the fixture is committed JSON, not a console transcript.

The live harness comes after the library cases. `cargo xtask live run walk grid` with a Jolt switch on the C# region, and the same scenario against Ferroan, once the character exists. Until then, walk stays `--physics basicphysics` and is a stand-in test, not a Jolt test. A second live case is two avatars logging in at one point on a Jolt region, for the arrival ray. Vehicle and collision live runs are the gate, after the oracle cases for those behaviours are green. Live runs take the shared live lock. This planning task does not start one.

### Risks

- **Jolt version.** 5.4-generation joltc on the C# side, Jolt 5.6 in `oxijolt-sys` 1.5. Friction and contact changed between them. Bit-identical traces are not the acceptance test. The open question is whether the maintainer wants that split written down as a decision.
- **Floating point.** `FrameTime` is a C# `float`. The accumulator uses `double` for the carry and `float` for the step length (`(float)(1.0/45.0)`). Gravity is the float −9.80665. A Rust port that promotes a step to `f64` and comes back will drift. The first box row should use the same widths the C# step uses, and say so in the test.
- **Threading.** C# runs simulate on the heartbeat thread and Jolt workers inside `Update`. Ferroan runs the frame and Jolt on two threads ([D6](../DECISIONS.md)). The frame must wait for the heartbeat's steps before it reads poses. A one-tick lag shows up as the avatar terse update trailing the physics position, which is the bug `R29` already hit with the stand-in. The job pool stays at one update at a time. Two regions in one process are not the current Ferroan shape. If that shape ever changes, this native still forbids two `PhysicsSystem` updates at once.
- **Step order.** Shipped order is linkset weld, camera rays, pending activation, then for each 1/45 s step: vehicles, script forces, targets, character, solver. Reports once per heartbeat, contacts merged. An oracle that steps once per heartbeat (`PhysicsStepRate = 0`) will not match a region running the default. The first fixture uses the default.
- **Character versus body.** Characters step before the solver and are not in it. A test that expects an avatar to be a dynamic capsule will not match.
- **Mesher pixels.** Sculpt images go through SkiaSharp in C#. Ferroan cannot add Skia ([D8](../DECISIONS.md)). Sculpt vertices will differ by the resampler. Convex cells (`PrimHullCells`) are what a physical cut prim uses. Substituting parry3d V-HACD changes hull count, mass and contacts.
- **InWorldz vehicle defaults.** `VehicleBehaviour = InWorldz` with `VehiclePresets = Documented` is the shipped combination. A port that follows only the wiki will fail the boat and car telemetry.

## 4. Queue items

Each row is about a day. Dependencies are earlier Phase 3 rows plus the Phase 2 rows that already landed. Evidence is a capture or an oracle case against Pin8 Jolt, unless the row is only the crate bind.

The C# scene the oracle builds uses `[Startup] physics = Jolt` and the shipped `[Jolt]` defaults, heartbeat 0.0909, gravity −9.80665, `PhysicsStepRate` left unset so the code default of 45 applies.

| Id | Title | C# source | Evidence | Depends on |
| --- | --- | --- | --- | --- |
| P3.1 | Bind `oxijolt-sys` in `ferroan-physics` | `Jolt.Backend.csproj` 31–38. `JoltNative.cs` 1–338 (one native per process, recorded build, single precision). `JoltPhysicsBackend.cs` 12 and `Initialize` (the `Foundation.Init(false)` path). | The crate builds on the three CI platforms. It records the Jolt version string from the 5.6 native. A test creates a physics system and drops it. Every `unsafe` block has a `SAFETY` comment. No other crate depends on `oxijolt-sys`. | None. [D6](../DECISIONS.md), [D8](../DECISIONS.md), [D9](../DECISIONS.md). |
| P3.2 | Static terrain and one falling box | `JoltScene.SetTerrain` 3552–3613. `CreateHeightFieldShape` 1842, backend `SetTerrain` 4515, `CreateBoxShape` 1688, `CreateBody` 2034, `Step` 5022–5106. `SimulateSubsteps` 3096–3156. `SubstepAccumulator.Advance` 52–75. `JoltConfig` gravity 38, `DefaultPhysicsStepRate` 194, `EffectivePhysicsStepRate` 361–379, `CollisionStepsAt` 409–412. | Oracle: a flat 256 m heightmap and one dynamic box, released from a known height. JSON of position and velocity per heartbeat until the box sleeps, plus the step count each heartbeat took. Ferroan replays that file. The first run writes the tolerance it needed. | P3.1. The float layout is the one `P2.0e` already stores (`y * width + x`). The frame is not required yet. |
| P3.3 | Physics thread beside the frame | `Scene.Update` 1818–1823 and `SceneGraph.UpdatePhysics` 222–224, for the "physics, then presence" order. `JoltScene.Simulate` 2786–2796 for the entry. | The frame posts one heartbeat, the physics thread runs P3.2's step, and the frame reads the box pose only after the join. A unit test shows a refused overlap: the frame does not sample mid-step. Avatars still use the stand-in. The box is not an avatar. | P3.2, `P2.1a` (frame thread), `P2.2b1` (stand-in call site, left in place for avatars). |
| P3.4 | Analytic box, sphere and cylinder | `JoltScene` fast path 1186–1275. `PrimHasNoCuts` 2224–2228. Backend shape constructors 1688–1717. | Oracle: one static and one dynamic of each shape, posed, never colliding. Pose and mass match. A cut, hollow or tapered prim is not in this row. | P3.2 |
| P3.5 | Static and physical body flags | `JoltPrim` create path through the inert-then-activate handoff, `JoltScene.DrainPendingActivation` 2748–2766. Phantom and volume-detect layers, `JoltPrim.cs` 50–54 and 242–243. Selected body held still, 1857–1891. | Oracle: a static box stays. A physical box falls (P3.2's trace). A phantom box falls through the static one. A volume-detect box overlaps and records a sensor contact. A selected physical box does not fall. | P3.4, `P2.4a` (stored prims have these flags). |
| P3.6 | Uncut-prim mass, friction and gravity scale | `JoltPrim` buoyancy and gravity factor 1607–1818. `HalcyonBasicForces.LegacyBuoyancy` 97–104. Backend `SetBodyGravityFactor` 2758, friction 2595, restitution 2611. | Oracle: three boxes, buoyancy 0, 0.5 and 1, dropped together. Vertical velocity per heartbeat. The 0.977 value under InWorldz behaviour becomes 1. | P3.5. Water height from `P2.4c` is recorded and unused until hover. |
| P3.7 | Profile and path mesh for a cut box | `Meshmerizer.cs` the prim entry the cook calls. `PrimMesher.cs` profile and path (the `Profile`, `Path` and `PrimMesh` types, 608–1453 and 1453–end). `CookMeshShapeCore` 1546–1711 for a non-physical triangle mesh. | Oracle: one cut box, non-physical. Vertex and index counts, and a downward ray that hits the cut. A Rust mesher that only returns a bounding box fails the ray. | P3.4. BSD headers on the mesher port ([PORTING.md](../PORTING.md)). |
| P3.8 | One convex hull for a physical cut box | The physical branch of `CookMeshShapeCore` 1710–1711. | Oracle: the same cut box, physical, dropped onto flat terrain. It has non-zero mass and comes to rest. The triangle-mesh path (volume 0) is the failure this row exists to catch. | P3.7, P3.6 |
| P3.9 | Convex cells for a physical concave prim | `PrimHullCells.cs` 68–451. `JoltScene` cells path 1869–1910. `PrimProfileRings.cs`. | Oracle: a hollow box, physical. Hull count, mass and rest position. A single hull of the outer box is a failed match, because the hole is the point. | P3.8 |
| P3.10 | Sculpt | `SculptMesh.cs`, `SculptMap.cs`. Cook fallback when the image is missing, `JoltScene.cs` 1610–1612, and the inside-out flip at 1649. | Oracle: one committed sculpt image, already decoded to raw pixels by the C# mesher so the fixture does not need Skia in Rust. Vertex positions within a tolerance the row records. A missing image stays a box. | P3.7. Decode the image with the Rust crates the repo already allows (`j2k`, tiny-skia). No SkiaSharp. |
| P3.11 | Mesh-asset hull list | `Mesh.cs` `ConvexHulls`. `JoltScene` shared shape key 1314 and the mesh branch of `CookMeshShapeCore`. `[Jolt] HonourConvexMesh` default false, `JoltConfig.cs` 106–108. | Oracle: one mesh asset with a hull list. A non-physical copy is a triangle mesh. A physical copy uses the asset hulls. A second prim of the same asset shares the cook. An asset that has not arrived is a box, then recooks. | P3.8, `P2.5a` (stored mesh prims and their assets). |
| P3.12 | Linkset weld | `JoltPrim.link` / `delink` 1991–2028. `RebuildCompoundNow` 2070. `DrainDirtyLinksets` 2768–2784. Mass sum around `JoltPrim.cs` 1345. | Oracle: root plus two children, linked, dropped. One body, mass the sum, collision on the struck child. Unlink restores three bodies. Matches `linktest` / `unlinktest`. | P3.5, P3.9 |
| P3.13 | Avatar capsule, stand, walk, fly | `JoltCharacter.cs` 1–200 (capsule, mass, drain). `StepCharacter` flying and walking branches 4121–4191. Speed factors `JoltConfig.cs` 172–182. | Oracle: one avatar, scripted intent, no prims. Walk, run, fly, stop. Feet position per heartbeat against the Jolt region, not against `fixtures/walk/walk.cap` (that file is BasicPhysics). | P3.3, P3.2, `P2.2a` (intent), `P2.2b2` (terse thresholds stay). |
| P3.14 | Fall, land, jump, slope, stairs | `StepCharacter` 4138–4249. `LooseObjectInStep` 3228. Stair settings 4215–4227. | Oracle: drop onto flat terrain, jump, walk a 0.2 m step, walk a slope inside the max angle and one steeper. Support flag and feet Z per heartbeat. | P3.13 |
| P3.15 | Remove the walking stand-in | `walk.rs` is the code that goes. C# reference is the Jolt character, not BasicPhysics. | `cargo xtask live run walk grid` with the C# region on Jolt and Ferroan on `ferroan-physics`. The basicphysics capture remains the history of Phase 2. Avatars no longer walk through a static box. | P3.14, P3.5. [D38](../DECISIONS.md). |
| P3.16 | Arrival landing ray | `ScenePresence.MakeRootAgent` 1543–1602. `SupportsRaycastWorldFiltered` and `RaycastWorld` 2247–2284. Filter `PrimsNonPhantomAgents`. | Oracle: a static box under the arrival point, ray down, hit point. Live, after that: two avatars logging in at the same point on a Jolt C# region and on Ferroan. The second stands on the first. | P3.13, P3.5, `P2.0d3` / `P2.0h` (the scene landing rule this ray joins). |
| P3.17 | Ray budgets and the sit ray | `JoltScene.cs` 2258–2293. `JoltPhysicsBackend.RayCasts.cs`. `RayCastTimeExceededException.cs`. | Oracle: a ray that hits, a ray that misses, a ray refused by an empty budget, and `RaycastActor` against one child of a linkset. | P3.16, P3.12 |
| P3.18 | Collision reports | `DispatchContacts` 3237–3416. `CollisionFrameTracker.cs`. `StillTouchingAsleep` 3537–3549. `GetTopColliders` 3705–3718. | Oracle: drop a box onto a platform (`collidetest`) and onto link 3 of a physical linkset (`collidelinktest`). Heartbeat, local ids, phase. A sleeping pair does not emit end. Avatar feet normal becomes the collision plane. | P3.12, P3.14. Script delivery is Phase 4. This row stops at the event the actor stores. |
| P3.19 | Vehicle parameter store | `VehicleController` `ProcessTypeChange` 462, parameters 513–740, flags 799. `VehicleProperties.cs` 82–184. `VehicleConstants.cs`. `JoltPrim` 2335–2382. | Oracle: type, then each float, vector and flag, read back. Documented presets plus InWorldz behaviour, as the ini defaults. No motion yet. | P3.5 |
| P3.20 | Vehicle step, boat | `VehicleController.Step` 1018, `StepOnce` 1132, `StepCore` from 1175 through buoyancy and the linear motor. `HalcyonVehicleLaws.cs`. `VehicleStepProvider.cs`. `SetWaterLevel` 3691–3694. | Oracle: a boat on water, buoyancy and a linear motor, telemetry per heartbeat. The `boattest linear` case. | P3.19, P3.6, P3.3. Water height from `P2.4c`. |
| P3.21 | Vehicle hover and vertical attraction | The hover and attract part of `StepCore`, and `VehicleSpring.cs`. Flags `HoverWaterOnly`, `HoverTerrainOnly`, `HoverGlobalHeight`, `HoverUpOnly`. | Oracle: boat hover over water, balloon hover at a global height. Hover error per heartbeat. | P3.20 |
| P3.22 | Car and sled | The ground-vehicle branch of `StepCore`, `VehicleGroundGravityFactor`, sled friction 0. `SledSlideScenario.cs` as a case list. | Oracle: `cartest` linear and steer, `sledtest` slide. Rest and a steered turn. | P3.21, P3.2 (slope) |
| P3.23 | Airplane and balloon motors | Angular motor, banking, deflection, `MouselookSteer` / `MouselookBank` given a fixed camera vector (no viewer). | Oracle: `planetest` thrust and bank, `balloontest` lift. | P3.21 |
| P3.24 | Motorcycle, sailboat, saved `VehicleData` | `ProcessTypeChange` cases 482–483. `JoltPrim.SetVehicle` 2382. | Oracle: both extra types accept their LSL codes. A saved record restores flags and does not start the motors. | P3.23, P3.12 (a vehicle linkset) |
| P3.25 | Sit | `JoltSitAvatar.cs`. `RaycastActor` sit query. `[Jolt] PhysicsSit`. | Oracle: `sittest` and `sittarget`. The seated offset matches. Stand returns the avatar to the character controller. | P3.17, P3.14 |
| P3.26 | Attachment forces on the wearer | `JoltCharacter.cs` 461–518. The constant-force branch of `StepCharacter` 4099–4136. | Oracle: a force, an impulse and a buoyancy applied to the avatar as an attachment would. The avatar's velocity changes. No worn prim body appears. | P3.14, P3.6. The script calls are Phase 4. |
| P3.27 | Read-only `jolt` commands | `JoltScene.cs` 626–647. `CapacityReport.cs`. `JoltMetrics.cs`. The `vehiclestatus` line in `JoltPrim.cs` 193–198. | The same text for capacity, a terrain probe and vehicle status, on a scene the oracle built. The test-command assembly is not ported. It stays the C# driver. | P3.2, P3.13, P3.20 |
| P3.28 | Gate captures | The live harness, not a new C# file. | `walk`, a dropped box with a collision, and one vehicle, C# Jolt beside Ferroan. Firestorm stays the maintainer's check, as [TESTING.md](../TESTING.md) says. | P3.15, P3.18, P3.22 |

`P3.2` is the smallest slice that can fail a real compare: the native is bound, the terrain is the heightfield, and one box falls under the shipped step clock.

Rows that will probably split when someone reads them: `P3.7` (`PrimMesher.cs` is 2,449 lines; a torus can become its own row), `P3.9`, `P3.20` (`StepCore` is the long function), and `P3.23`.

## 5. Open questions

1. **Jolt version.** PLAN names Jolt 5.6 through `oxijolt-sys` 1.5. Pin8 links JoltPhysicsSharp 2.19.1 and JoltPhysics.Native 1.0.4, a joltc from 10 October 2025 in the 5.4 generation. Jolt 5.6 changes friction. The rows above keep 5.6 and compare with tolerances. A same-version native would mean either moving the C# module forward or pointing the oracle at an older joltc than `oxijolt-sys` ships. That choice wants a decision if the gate is to say "same results" with a number attached.

2. **Convex decomposition.** PLAN's building-block table names parry3d's V-HACD. The Jolt module never calls `ConvexDecompositionDotNet`. Physical concave prims use `PrimHullCells`. Porting `PrimHullCells` is what the oracle can check. V-HACD would be a different shape. parry3d is Rust, so it does not need a [D8](../DECISIONS.md) exception. The question is whether the building-block line should be corrected to `PrimHullCells` when the first hull row lands.

3. **Sculpt pixels.** `SculptMap.cs` uses SkiaSharp. [D8](../DECISIONS.md) does not allow that native library. The sculpt row decodes with `j2k` and tiny-skia and expects a tolerance. Confirm that is acceptable, or name a pure-Rust resampler that should be matched instead.

4. **Safe `oxijolt` crate.** crates.io also publishes `oxijolt`, a safe wrapper that calls `oxijolt-sys`. [AGENTS.md](../../AGENTS.md) says `ferroan-physics` is the only crate that calls the native. The rows depend on `oxijolt-sys` directly and put the safe wrapper in `ferroan-physics`. Confirm the safe crate stays out of `Cargo.toml`.

5. **Vehicle behaviour.** Shipped defaults are `VehiclePresets = Documented` and `VehicleBehaviour = InWorldz`, with `VehicleGroundGravityFactor = 1`. The gate follows those. Say if the maintainer wants `VehicleBehaviour = SecondLife` for Ferroan's region ini instead. That is a different set of traces.

6. **Diagnostics in the product.** The read-only `jolt` commands are in `P3.27`. The test commands stay on the C# side as the oracle. Say if any test command should also exist on the Ferroan console.

7. **One region per process.** The current native cannot update two physics systems at once. Ferroan is one region per process, so one physics system on the Jolt thread is enough. A future multi-region process needs a joltc with a temp allocator per system, which none of the C# recorded builds have, or a queue that runs one `Update` at a time. No decision is needed while the process hosts one region.
