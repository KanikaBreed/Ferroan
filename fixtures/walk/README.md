# Walking step fixture

`basicphysics-steps.json` is one `BasicScene.Simulate` trace at pin `c401bceecd`. The step is `FrameTime` 0.0909. The avatar box is 2.1.

`tools/walk-oracle` checks that file. It parses each number as `float` and compares `Simulate` bit for bit. It does not rewrite the file, and `cargo xtask check` does not build it.

```
dotnet run --project tools/walk-oracle -- verify fixtures/walk/basicphysics-steps.json
```

Set `FerroanRef` to the reference checkout when it is not `~/Git/reference`. The oracle compiles against `OpenSim.Region.PhysicsModules.SharedBase.dll` in that project's `bin/Debug/net10.0` directory. Building BasicPhysics writes it.

`flat_walk`, `fall`, `fly_hold` and `fly_snap` use a 256 m region and flat ground 21. `border` and `outside` use a 16 m region and the same ground. `slope` uses a 16 m region whose cell (2, 3) is 30 and whose other cells are 21. `no_map` has no heightmap: it starts at (4, 5, 26) with no velocity and lands at the single stored row. A `*_flying` key is 1 when that case is flying. The first row of each other case is the actor before `Simulate`. Column 7 of a later row is ground contact.

A kinematic sample may finish outside the region. The presence tick's no-neighbour stop is not part of this file. The live walk from a running `--physics basicphysics` region is AWAITING VALIDATION. That capture, with `walk: pass`, is evidence for P2.2b2.
