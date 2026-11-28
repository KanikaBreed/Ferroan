# Jolt oracle

`cargo xtask jolt substeps` writes `fixtures/jolt/substeps.json`.
`cargo xtask jolt substeps-verify` rebuilds that document and requires the same bytes.

The tool compiles only
`Source/OpenSim.Region.PhysicsModules.Jolt/SubstepAccumulator.cs` from the
scratch pin into this assembly and runs that class. It does not copy the file
into Ferroan, it does not build the Jolt project, and it does not start a
region. The pin is `FERROAN_REF` or `~/Git/reference`. The project sets
`UseSharedCompilation` to false. `FERROAN_REF` stays read-only.

`cargo xtask jolt generate` writes `fixtures/jolt-matched/drop.json`.
`cargo xtask jolt verify` runs that same `drop` again and requires the
same bytes. The project is `JoltMatchedOracle.csproj` in this directory.
It reuses the P3.57 recorder (`RecordOracle.cs`): `Harness.Find("drop")`
and `Harness.Run`, with `PhysicsRateHz` 45. The heartbeat stays 1/11 s.
The JSON names that heartbeat, the per-heartbeat step count, the mesher
type `Harness.Run` assigned, the clock ticks, and `DeterministicMode`
from the run's config. The baseline is `det-886e0886-linux-x64`. Ordinary
`cargo xtask check` does not run it, and neither does the live workflow.
It does not link `ferroan-physics` and it does not start a region.

`jolt substeps` stays the accumulator only. It does not build
`JoltMatchedOracle.csproj`.
