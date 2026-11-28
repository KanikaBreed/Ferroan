# Jolt config oracle

`cargo xtask jolt config` writes `fixtures/jolt-config/`.
`cargo xtask jolt config-verify` reads those files again and requires the same bytes.

`cargo xtask jolt file` writes `fixtures/jolt-file/`.
`cargo xtask jolt file-verify` reads those files again and requires the same bytes.

The config command calls `JoltConfig.FromConfig`, `EffectivePhysicsStepRate`,
`EffectivePresets` and `JoltScene.TestCommandsEnabled` on the pinned Jolt
module. The file command calls private `JoltScene.NoteDilationSample`
(`JoltScene.cs` 2928-2942) by reflection on `new JoltScene()` and reads the
private dilation fields back. It does not call `Initialise`. It also compiles
`JoltCharacter` and writes `fixtures/jolt-file/character`. A later row can add
its own fixture in this producer. The producer compiles the Jolt project, so
the Pin8 files that project contains are the files those rows name.

Neither command copies those sources, loads joltc, or starts a region. The
project references the Jolt project so the call is the module's own method.
Build output goes under `.scratch/jolt-config-oracle/`. The pin is
`FERROAN_REF` or `~/Git/reference`, and that checkout stays read-only. The
project sets `UseSharedCompilation` to false. Temporary ini files for the
synthetic config cases go under `.scratch/tmp`.

Synthetic config cases are ini text. The shipped case loads
`RegionServerDefaults.ini` and `RegionServer.ini.example` from the pin with
Nini, the same files the region loader reads. Ordinary tests compare the
committed results. The shipped case runs when the pin is present.

The file fixture is one ordered call list on two scenes. Scene A stores a 1
second step, ten 0.5 second steps and a 0.25 second step that wraps the
11-wide window. Scene B stores a 1 second step against one stopwatch tick, so
the dilation stays 1. Gap ticks of one frequency are exactly one second. A
one-tick gap is written as `frequency_divisor` because that quotient follows
the machine clock; the producer checks the stored double against
`1/Stopwatch.Frequency` and records the dilation float from `_timeDilation`.
Exact stored ratios are 1, 0.75, 0.625 at sample 3 and 0.5625 at sample 7.
The Rust test checks the fixture's structure and pins all twelve scene A
dilation values. `DilationWindow` replays the ratio. The same command calls
`Simulate` and the reporter's FPS transform for both `Normalized55FPS`
settings and writes `fixtures/jolt-file/stats`. A successful simulate
contributes 1. Eleven frames in one second show about 55 when the scale is
on. `SimPhysicsStepMs` stays 20. Ordinary check does not call the reporter.

The same file command calls internal `CookPrimShape` (`JoltScene.cs`
1229-1259) by reflection on `new JoltScene()` and records the constructor
the fast path chooses. A box, a sphere and a cylinder are each cooked once
static and once dynamic. The cylinder case is circle profile and straight
extrusion; `PrimitiveBaseShape.CreateCylinder()` is not that shape. The
recording backend stores the half extents, the sphere radius, or the
cylinder half height and radius. It does not call the native constructors.
A non-uniform sphere is `bbox(fallback)` with one box constructor, because
the mesher is null. `analytic_shape` replays the fast path and returns
nothing for that ellipsoid. The stock drop asset kind stays `analytic-box`.
Mass and a ray are on the stock harness recording, in
`fixtures/jolt-record/analytic-envelope.txt`, not in this no-native fixture.

The same file command calls public `SetTerrain` (`JoltScene.cs` 3552-3613)
on `new JoltScene()` and records the `(N+1)` field that method stores. The
backend records the span, the sample counts and the scale, and does not cook.
A 256 by 256 map becomes 257 by 257. The fixture stores a SHA-256 of those
little-endian output floats, the dimensions, and the edge and corner bits.
Smaller maps stay literal. The far edge repeats the last real sample.
Non-square maps pad to `max(width, height) + 1` by that same repeat. A side
past 4096 m reports `terrain.edge` and does not reserve the square.
A short map, a null map and a zero region size leave the field unset. Gravity,
prim damping and the `1/11` s step are the stock drop recording's bits.
`pad_heightfield` replays the fixture. This row does not compare a cooked ray.

The same file command calls `JoltCharacter` capsule, mass and stand-half,
`DesiredVelocity`, the jump latch and the ground flags through
`ApplyCharacterState`, and records the `CharacterDesc` a stub backend
received. The samples are those method results. A finite footprint keeps a
radius of at least 0.2 m. A constructor size that is NaN or infinite is
recorded by bits, and a 5e6 m footprint hits the 0.05 m half-height floor.
The replay compares NaN as NaN because the default NaN's bits are
platform-defined. Only `fixtures/jolt-file/character/result.txt` has NaN
results (capsule rows 21-24, size row 50, buoyancy row 57). The heightfield
fixture stores a NaN payload as hex bits.
`fixtures/jolt-file/gravity/matrix.txt` records a passed NaN payload by
bits. The note-dilation, config and drop replays stay bit-exact.
The default mass bits match `DefaultAvatarMass`. Walk and run leave Z
unscaled. Flight ignores always-run, for both signs of Z. The latch clears
when vertical velocity is above 0.5 or the avatar is not supported.
`KeepInRegion` records an allow-outside return, an invalid character and a
256 by 128 region. The descriptor's user data is the value the query marker
would copy; the body is not created. Heartbeat and native code are not
involved. A ray that would see the query marker, and the body-cap miss, wait
for the bind. The matched-native trace waits for the upgrade. The gravity
scale inside `StepCharacter` is not invoked; that multiply is P3.13b.
Default speed-factor and jump bits are the same bits as
`fixtures/jolt-record/drop/config.txt`. The capture still does not load joltc.

The same file command calls `HalcyonBasicForces.LegacyBuoyancy`
(`HalcyonBasicForces.cs` 103-104; cite lines 1-33) and `JoltPrim.Buoyancy`
and `VehicleType` (`JoltPrim.cs` 1784-1819, 2303-2332, 2755-2767 and
2794-2810). The prim is uninitialised, so its constructor does not cook a
shape. A recording backend stores `SetBodyGravityFactor` and ignores the
vehicle body's friction, damping, sleep, CCD and step-provider calls. The
0.977 boundary, a Second Life value that stays 0.98, the multiplier and its
clamp, a rejected non-finite value, a child write on the root and a vehicle
factor of 0 restored to the script factor are in
`fixtures/jolt-file/gravity/matrix.txt`. Water is not used. The Rust replay
compares every write by bits. Ordinary tests do not run this command.

No native code is started. The fixture records the same proof at
`before_construct`, after construction and after the calls:
`JoltNative.s_loaded` is null,
`JoltNative.s_handle` is zero, `JoltPhysicsBackend.s_foundationRefCount` is
zero, and `joltc` is not mapped into the process. `EnsureLoaded` is the
loader's only write of those `JoltNative` fields. `Foundation.Init` runs only
from `JoltPhysicsBackend.Initialize`, which increments the refcount in the
same lock, and `Init` then calls `JPH_Init`. A zero refcount and an unmapped
`joltc` mean that call did not happen. The project reference may load the
managed assembly that references `JoltPhysics.Native`. Linking joltc is not
the test. `cargo xtask jolt generate` and `jolt verify` are the matched-native
wrapper (P3.0), not this producer.

`cargo xtask jolt record` writes `fixtures/jolt-record/`.
`cargo xtask jolt record-verify` runs the pinned harness again and requires
the same bytes. The recorder is `JoltRecordOracle.csproj` in this directory.
`JoltConfigOracle.csproj` does not compile it, and the recorder does not
compile the config or file sources. The config and file commands still do
not load joltc.

The record command runs `Harness.Run` in-process for the harness `drop`.
The samples are the harness `Sample` fields. `ToCsv` is not the trace.
The harness awake-body count is written as `awake_bodies`. `awake` is
`IsBodyAwake` for the dropped body after that step.
The heartbeat is `1/11` s (`RateHz` 11). `PhysicsStepRate` 0 is one step
per heartbeat, read from `JoltScene.Substeps` when that accumulator exists
and from `JoltScene.StepCount` when it does not. The host subscribes to the
actor and records collision updates. The stock drop attaches no step
provider. After the samples, events and step marks are stored, the proof
attaches `ScriptForceStepProvider` to the dropped box and replaces that
entry's `Provider` field with a recording wrapper. One extra simulate
forwards to the stock provider. The field is restored, and disposing the
attachment leaves no provider and no step listener. The wrapper does not
replace the entry: detach removes that entry by reference.

Each fixture records the native file name, size and sha256, the harness
commit, the heartbeat, the step count, the config, the warm-up count, the
body ids, the clock and the effective materials. Asset bytes, the cold
sculpt-cache result, the first heartbeat and a hash of each channel are in
the same provenance. `material_pins`, `use_defaults`, `shipped_config`,
`warm_up`, `simulator_prims` and `region_mesher` are read from the options,
`MaterialPins.Enabled` and whether the run installed a mesher.
`files_before` is the sculpt-cache count before the run. `provider_calls`
counts stock `OnStep` calls the seam forwarded. `first_heartbeat` is the
first heartbeat index the host observed. A wrong-clock file and a
wrong-collider file change one captured number each. Verify writes each
file over its channel and requires the tree compare to reject it. The
anchor is the pin's deterministic linux-x64 joltc (origin `Rebuilt`, 2,701,768
bytes, SHA-256 `27EFC48C…288B`); `JoltRecordOracle.csproj` imports the pin's
`JoltNative.targets`, which copies that file over the package's. The baseline
name is `det-886e0886-linux-x64`, which must equal `BaselineCheck.KeyFor` of
the loaded build. Any other native prints `SKIPPED` and exits 3, and that
result does not pass.
The command clears `JOLT_HARNESS_PHYSICS_RATE` before it builds the harness
options. It does not patch the pin and it does not start a region.

Build output goes under `.scratch/jolt-record-oracle/`. The project sets
`UseSharedCompilation` to false. The live workflow runs `record-verify`.
