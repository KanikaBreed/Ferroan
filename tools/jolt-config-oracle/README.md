# Jolt config oracle

`cargo xtask jolt config` writes `fixtures/jolt-config/`.
`cargo xtask jolt config-verify` reads those files again and requires the same bytes.

The tool calls `JoltConfig.FromConfig`, `EffectivePhysicsStepRate`,
`EffectivePresets` and `JoltScene.TestCommandsEnabled` on the pinned Jolt
module. It does not copy those sources, it does not load joltc, and it does
not start a region. The project references the Jolt project so the call is
the module's own reader. Build output goes under `.scratch/jolt-config-oracle/`.
The pin is `FERROAN_REF` or `~/Git/reference`, and that checkout stays
read-only. The project sets `UseSharedCompilation` to false. Temporary ini
files for the synthetic cases go under `.scratch/tmp`.

Synthetic cases are ini text. The shipped case loads
`RegionServerDefaults.ini` and `RegionServer.ini.example` from the pin with
Nini, the same files the region loader reads. Ordinary tests compare the
committed results. The shipped case runs when the pin is present.
