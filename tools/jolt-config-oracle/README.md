# Jolt config oracle

`cargo xtask jolt config` writes `fixtures/jolt-config/`.
`cargo xtask jolt config-verify` reads those files again and requires the same bytes.

`cargo xtask jolt file` writes `fixtures/jolt-file/`.
`cargo xtask jolt file-verify` reads those files again and requires the same bytes.

The config command calls `JoltConfig.FromConfig`, `EffectivePhysicsStepRate`,
`EffectivePresets` and `JoltScene.TestCommandsEnabled` on the pinned Jolt
module. The file command calls private `JoltScene.NoteDilationSample`
(`JoltScene.cs` 2928-2942) by reflection on `new JoltScene()` and reads the
private dilation fields back. It does not call `Initialise`. A later row adds
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
Exact stored ratios in the fixture are 1 and 0.75. The full-window ratios are
the method's own floats. Ordinary tests replay the committed file.

No native code is started. The fixture records the same proof before the
scene is constructed and after the calls: `JoltNative.s_loaded` is null,
`JoltNative.s_handle` is zero, `JoltPhysicsBackend.s_foundationRefCount` is
zero, and `joltc` is not mapped into the process. `EnsureLoaded` is the
loader's only write of those `JoltNative` fields. `Foundation.Init` runs only
from `JoltPhysicsBackend.Initialize`, which increments the refcount in the
same lock, and `Init` then calls `JPH_Init`. A zero refcount and an unmapped
`joltc` mean that call did not happen. The project reference may load the
managed assembly that references `JoltPhysics.Native`. Linking joltc is not
the test. `cargo xtask jolt generate` and `jolt verify` stay on P3.0.
