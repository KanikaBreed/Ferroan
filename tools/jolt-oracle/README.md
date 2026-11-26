# Jolt oracle

`cargo xtask jolt substeps` writes `fixtures/jolt/substeps.json`.
`cargo xtask jolt substeps-verify` rebuilds that document and requires the same bytes.

The tool compiles only
`Source/OpenSim.Region.PhysicsModules.Jolt/SubstepAccumulator.cs` from the
scratch pin into this assembly and runs that class. It does not copy the file
into Ferroan, it does not build the Jolt project, and it does not start a
region. The pin is `FERROAN_REF` or `~/Git/reference`. The project sets
`UseSharedCompilation` to false. `FERROAN_REF` stays read-only.

P3.0 still owns the harness wrapper and `cargo xtask jolt generate` /
`verify`. This command is the accumulator only.
